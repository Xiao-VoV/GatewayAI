//! observability 用例：请求日志写入（同步/旁路 + 优雅退出 flush）。
//!
//! 设计见 `设计文档（暂未开源）` §3.1/§5：
//! - `record_request_log`：单次幂等写入（测试/需确认时 await）；
//! - `record_request_log_detached`：spawn + 有界重试（立即/200ms/1s，共 3 次）+ in-flight，
//!   失败仅 `tracing::warn!`，**不影响主请求**；
//! - `flush`：main 优雅退出等待 in-flight 归零（≤timeout）；
//! - 写入前经 `sanitize` 二次防御；非法 `request_id`/`trace_id` 直接丢弃（不写脏数据）。
//!
//! 管理台：请求日志列表/详情（S5）、审计列表（S5.1）、requests/audit CSV 导出（S5.1）；
//! S5.1 另提供审计写入与 request_logs/audit_logs 保留清理（`run_cleanup_once`/`spawn_cleanup`）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::platform::{error::AppError, state::AppState};

use super::{
    csv,
    model::RequestLogEvent,
    repo,
    sanitize::{
        MAX_MODEL_NAME_CHARS, MAX_PROTOCOL_LABEL_CHARS, is_valid_request_id, sanitize_audit_detail,
        sanitize_client_ip, sanitize_error_code, sanitize_key_prefix, sanitize_method,
        sanitize_optional_text, sanitize_path, sanitize_user_agent,
    },
};

/// 旁路写入总尝试次数（立即/200ms/1s）。
const WRITE_ATTEMPTS: usize = 3;
const RETRY_DELAYS_MS: [u64; WRITE_ATTEMPTS] = [0, 200, 1_000];

/// 已 spawn 但未落库的用例数（flush 等待用）。
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

struct InFlightGuard;

impl InFlightGuard {
    fn acquire() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 单次写入（幂等；不隐式 spawn；测试/需要确认时 await）。
#[allow(dead_code)] // proxy/catalog 同步确认路径接入后移除（当前生产走 detached）
pub async fn record_request_log(state: &AppState, event: RequestLogEvent) -> Result<(), AppError> {
    let record = prepare_record(&event)?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(AppError::database_unavailable)?;
    repo::insert_request_log(pool, &record).await
}

/// 数据面默认入口：fire-and-forget + 有界重试 + in-flight 计数；失败只记日志。
///
/// sanitize/校验在 spawn 前完成：非法事件立即丢弃并告警，不产生无意义的 DB 重试。
pub fn record_request_log_detached(state: &AppState, event: RequestLogEvent) {
    let record = match prepare_record(&event) {
        Ok(record) => record,
        Err(error) => {
            tracing::warn!(
                request_id = %event.request_id,
                %error,
                "request log dropped: invalid event"
            );
            return;
        }
    };
    let request_id = record.request_id.clone();
    let state = state.clone();
    // 在 spawn 前 +1，避免调用方立刻 flush 时漏掉尚未被调度的任务。
    let guard = InFlightGuard::acquire();
    tokio::spawn(async move {
        let _guard = guard;
        write_with_retry(&state, &record, &request_id).await;
    });
}

/// 优雅退出：等待 in-flight 写入归零；超时返回 false（调用方可忽略，行为仍是尽力排空）。
#[allow(dead_code)] // main.rs 优雅退出接入后移除
pub async fn flush(timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if IN_FLIGHT.load(Ordering::SeqCst) == 0 {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 事件 → `request_logs` 行：sanitize + 约束兜底归一化。
///
/// 非法 `request_id`/`trace_id` → 校验错误（detached 侧丢弃该条并告警）。
fn prepare_record(event: &RequestLogEvent) -> Result<repo::NewRequestLog, AppError> {
    if !is_valid_request_id(&event.request_id) {
        return Err(AppError::validation_field(
            "request_id",
            "request_id 非法（1–128 个 [A-Za-z0-9._:-]）",
        ));
    }
    if let Some(trace_id) = &event.trace_id
        && !is_valid_request_id(trace_id)
    {
        return Err(AppError::validation_field(
            "trace_id",
            "trace_id 非法（1–128 个 [A-Za-z0-9._:-]）",
        ));
    }

    // 约束兜底：ttft 仅流式、status_code 100–599、attempt_count 1–5、非负延迟/响应大小。
    let ttft_ms = if event.is_stream {
        event
            .ttft_ms
            .filter(|value| *value >= 0)
            .map(|value| value.min(i64::from(i32::MAX)) as i32)
    } else {
        None
    };
    let status_code = (100..=599)
        .contains(&event.status_code)
        .then_some(event.status_code);
    let latency_ms = event.latency_ms.clamp(0, i64::from(i32::MAX)) as i32;
    let response_size = event.response_size.filter(|value| *value >= 0);
    let error_code = match &event.error_code {
        Some(code) if !code.trim().is_empty() => Some(sanitize_error_code(code)),
        _ => None,
    };

    Ok(repo::NewRequestLog {
        id: uuid::Uuid::new_v4(),
        request_id: event.request_id.clone(),
        trace_id: event.trace_id.clone(),
        user_id: event.user_id,
        device_id: event.device_id,
        key_id: event.key_id,
        key_prefix: event.key_prefix.as_deref().and_then(sanitize_key_prefix),
        method: sanitize_method(&event.method),
        path: sanitize_path(&event.path),
        protocol: event.protocol.as_str(),
        upstream_protocol: sanitize_optional_text(
            event.upstream_protocol.as_deref(),
            MAX_PROTOCOL_LABEL_CHARS,
        ),
        adapter: sanitize_optional_text(event.adapter.as_deref(), MAX_PROTOCOL_LABEL_CHARS),
        model_alias: sanitize_optional_text(event.model_alias.as_deref(), MAX_MODEL_NAME_CHARS),
        upstream_model: sanitize_optional_text(
            event.upstream_model.as_deref(),
            MAX_MODEL_NAME_CHARS,
        ),
        provider: sanitize_optional_text(event.provider.as_deref(), MAX_MODEL_NAME_CHARS),
        provider_id: event.provider_id,
        is_stream: event.is_stream,
        fallback: event.fallback,
        attempt_count: event.attempt_count.clamp(1, 5),
        status_code,
        latency_ms,
        ttft_ms,
        response_size,
        error_code,
        client_ip: sanitize_client_ip(event.client_ip.as_deref()),
        user_agent: sanitize_user_agent(event.user_agent.as_deref().unwrap_or_default()),
        created_at: event.created_at,
    })
}

/// 旁路写入：按 `RETRY_DELAYS_MS` 重试，全部失败仅告警。
async fn write_with_retry(state: &AppState, record: &repo::NewRequestLog, request_id: &str) {
    let mut last_error: Option<String> = None;
    for (attempt, delay_ms) in RETRY_DELAYS_MS.iter().enumerate() {
        if *delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
        }
        match insert_once(state, record).await {
            Ok(()) => return,
            Err(error) => {
                if attempt + 1 < WRITE_ATTEMPTS {
                    tracing::warn!(
                        request_id = %request_id,
                        attempt = attempt + 1,
                        %error,
                        "request log write failed; retrying"
                    );
                }
                last_error = Some(error.to_string());
            }
        }
    }
    if let Some(error) = last_error {
        tracing::warn!(
            request_id = %request_id,
            attempts = WRITE_ATTEMPTS,
            error = %error,
            "request log write dropped after retries"
        );
    }
}

async fn insert_once(state: &AppState, record: &repo::NewRequestLog) -> Result<(), AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(AppError::database_unavailable)?;
    repo::insert_request_log(pool, record).await
}

// ---------- S5 管理查询 ----------

use chrono::Duration as ChronoDuration;

use super::model::{RequestListQuery, RequestLogDetail, RequestLogItem, RequestLogPage};
use super::repo::{RequestFilters, RequestLogRow};

fn db(state: &AppState) -> Result<&sqlx::PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
}

/// 列表：时间/身份/Key/模型/供应商/状态/trace 过滤 + 分页。
pub async fn list_requests(
    state: &AppState,
    query: &RequestListQuery,
) -> Result<RequestLogPage, AppError> {
    let pool = db(state)?;
    let (filters, limit, offset) = build_request_filters(query, ChronoDuration::hours(24))?;
    let (rows, total) = repo::list_request_logs(pool, &filters, limit, offset).await?;
    Ok(RequestLogPage {
        items: rows.into_iter().map(to_item).collect(),
        total,
        limit,
        offset,
    })
}

/// 请求日志筛选校验（列表/导出共用；`default_window` 为 from 默认窗口）。
fn build_request_filters(
    query: &RequestListQuery,
    default_window: ChronoDuration,
) -> Result<(RequestFilters, i64, i64), AppError> {
    let now = chrono::Utc::now();
    let from = query.from.unwrap_or(now - default_window);
    let to = query.to.unwrap_or(now);
    if from >= to {
        return Err(AppError::bad_request(
            "invalid_time_range",
            "from 必须早于 to",
        ));
    }
    if to - from > ChronoDuration::days(31) {
        return Err(AppError::bad_request(
            "invalid_time_range",
            "查询跨度不能超过 31 天",
        ));
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(AppError::validation_field(
            "limit",
            "limit 必须在 1–200 之间",
        ));
    }
    let offset = query.offset.unwrap_or(0);
    if !(0..=100_000).contains(&offset) {
        return Err(AppError::validation_field(
            "offset",
            "offset 必须在 0–100000 之间",
        ));
    }
    let status = status_bounds(query.status_class.as_deref(), query.status_code)?;
    let key_prefix = match query.key_prefix.as_deref().map(str::trim) {
        Some("") => None,
        Some(value) if !(4..=32).contains(&value.chars().count()) => {
            return Err(AppError::validation_field(
                "key_prefix",
                "key_prefix 长度必须在 4–32 之间",
            ));
        }
        Some(value) => Some(value.to_string()),
        None => None,
    };
    let model_alias = validate_filter_text(query.model.as_deref(), "model")?;
    let provider = validate_filter_text(query.provider.as_deref(), "provider")?;
    let trace_id = match query.trace_id.as_deref().map(str::trim) {
        Some("") => None,
        Some(value) if !(1..=128).contains(&value.chars().count()) => {
            return Err(AppError::validation_field(
                "trace_id",
                "trace_id 长度必须在 1–128 之间",
            ));
        }
        Some(value) => Some(value.to_string()),
        None => None,
    };
    let filters = RequestFilters {
        from,
        to,
        user_id: query.user_id,
        device_id: query.device_id,
        key_id: query.key_id,
        key_prefix,
        model_alias,
        provider,
        status_min: status.map(|(min, _)| min),
        status_max: status.map(|(_, max)| max),
        trace_id,
    };
    Ok((filters, limit, offset))
}

/// 详情：请求日志 + metering usage 块（metering 故障降级 null，不改变 200）。
pub async fn request_detail(
    state: &AppState,
    request_id: &str,
) -> Result<RequestLogDetail, AppError> {
    if !(1..=128).contains(&request_id.chars().count()) {
        return Err(AppError::validation_field(
            "request_id",
            "request_id 长度必须在 1–128 之间",
        ));
    }
    let pool = state
        .db
        .as_ref()
        .ok_or_else(AppError::database_unavailable)?;
    let row = repo::get_request_log(pool, request_id)
        .await?
        .ok_or_else(|| AppError::not_found("request_not_found", "请求记录不存在"))?;
    let item = to_item(row);
    let usage =
        match crate::modules::metering::service::get_usage_by_request_id(state, request_id).await {
            Ok(usage) => usage,
            Err(error) => {
                tracing::warn!(%error, request_id, "usage block unavailable; degrade to null");
                None
            }
        };
    Ok(RequestLogDetail { item, usage })
}

fn to_item(row: RequestLogRow) -> RequestLogItem {
    RequestLogItem {
        id: row.id,
        request_id: row.request_id,
        trace_id: row.trace_id,
        created_at: row.created_at,
        method: row.method,
        path: row.path,
        protocol: row.protocol,
        upstream_protocol: row.upstream_protocol,
        adapter: row.adapter,
        user_id: row.user_id,
        device_id: row.device_id,
        key_id: row.key_id,
        key_prefix: row.key_prefix,
        model_alias: row.model_alias,
        upstream_model: row.upstream_model,
        provider: row.provider,
        provider_id: row.provider_id,
        is_stream: row.is_stream,
        fallback: row.fallback,
        attempt_count: row.attempt_count,
        status_code: row.status_code,
        latency_ms: row.latency_ms,
        ttft_ms: row.ttft_ms,
        response_size: row.response_size,
        error_code: row.error_code,
        client_ip: row.client_ip,
        user_agent: row.user_agent,
    }
}

fn status_bounds(
    status_class: Option<&str>,
    status_code: Option<i32>,
) -> Result<Option<(i32, i32)>, AppError> {
    if status_class.is_some() && status_code.is_some() {
        return Err(AppError::bad_request(
            "invalid_status_filter",
            "status_class 与 status_code 不能同时使用",
        ));
    }
    if let Some(status_class) = status_class {
        return match status_class {
            "2xx" => Ok(Some((200, 299))),
            "3xx" => Ok(Some((300, 399))),
            "4xx" => Ok(Some((400, 499))),
            "5xx" => Ok(Some((500, 599))),
            _ => Err(AppError::bad_request(
                "invalid_status_filter",
                "status_class 只支持 2xx/3xx/4xx/5xx",
            )),
        };
    }
    if let Some(status_code) = status_code {
        if !(100..=599).contains(&status_code) {
            return Err(AppError::validation_field(
                "status_code",
                "status_code 必须在 100–599 之间",
            ));
        }
        return Ok(Some((status_code, status_code)));
    }
    Ok(None)
}

fn validate_filter_text(
    value: Option<&str>,
    field: &'static str,
) -> Result<Option<String>, AppError> {
    match value.map(str::trim) {
        Some("") => Ok(None),
        Some(value) if value.chars().count() > 100 => Err(AppError::validation_field(
            field,
            format!("{field} 不能超过 100 个字符"),
        )),
        Some(value) => Ok(Some(value.to_string())),
        None => Ok(None),
    }
}

// ---------- S5.1 审计写入 ----------

use serde_json::{Value, json};

use super::model::{AuditEvent, AuditItem, AuditListQuery, AuditPage};
use super::repo::{AuditFilters, AuditLogRow};

/// 导出 CSV 行数上限（超出返回 400 `export_too_large`，不静默截断）。
pub const MAX_EXPORT_ROWS: i64 = 100_000;
/// 审计列表默认/最大 limit（接口设计 §8.1：默认最新 100，limit ≤500）。
const AUDIT_DEFAULT_PAGE_LIMIT: i64 = 100;
const AUDIT_MAX_PAGE_LIMIT: i64 = 500;
/// 审计/导出默认时间窗口（最近 7 天）。
const AUDIT_DEFAULT_WINDOW_DAYS: i64 = 7;
/// 保留清理：每批行数 / 每 tick 最多批数。
const LOG_CLEANUP_BATCH: i64 = 10_000;
const LOG_CLEANUP_MAX_BATCHES: usize = 10;

/// 导出调用者（管理台 AdminSession + 请求元数据；供导出审计事件使用）。
#[derive(Debug, Clone)]
pub struct ExportActor {
    pub actor_id: uuid::Uuid,
    pub actor_email: String,
    pub client_ip: Option<String>,
    pub user_agent: String,
    pub request_id: Option<String>,
}

/// CSV 导出结果（route 负责设置 Content-Type/Disposition/no-store）。
#[derive(Debug, Clone)]
pub struct CsvExport {
    pub filename: String,
    pub body: String,
    pub row_count: usize,
    /// 过滤条件摘要（已进入审计 detail，经 sanitize_audit_detail 二次脱敏）。
    pub summary: Value,
}

/// 单次审计写入（同步；测试/关键路径用；跨模块审计 hook 下一轮接入）。
#[allow(dead_code)] // 跨模块 hook 下一轮接入后移除；本模块导出审计走 detached
pub async fn record_audit(state: &AppState, event: AuditEvent) -> Result<(), AppError> {
    let record = prepare_audit_record(&event)?;
    let pool = db(state)?;
    repo::insert_audit_log(pool, &record).await
}

/// 审计默认入口：fire-and-forget + 有界重试 + in-flight 计数；失败只记日志。
///
/// sanitize/校验在 spawn 前完成：非法事件立即丢弃并告警（不写脏数据）。
pub fn record_audit_detached(state: &AppState, event: AuditEvent) {
    let record = match prepare_audit_record(&event) {
        Ok(record) => record,
        Err(error) => {
            tracing::warn!(action = %event.action, %error, "audit dropped: invalid event");
            return;
        }
    };
    let action = record.action.clone();
    let state = state.clone();
    let guard = InFlightGuard::acquire();
    tokio::spawn(async move {
        let _guard = guard;
        write_audit_with_retry(&state, &record, &action).await;
    });
}

/// 事件 → `audit_logs` 行：校验 + sanitize（只追加；detail 脱敏 ≤4KB）。
fn prepare_audit_record(event: &AuditEvent) -> Result<repo::NewAuditLog, AppError> {
    let actor_type = validate_audit_actor_type(&event.actor_type)?;
    let action = validate_audit_action(&event.action)?;
    let resource_type = validate_audit_resource_type(&event.resource_type)?;
    let outcome = validate_audit_outcome(&event.outcome)?;
    let request_id = match event.request_id.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) if is_valid_request_id(value) => Some(value.to_string()),
        Some(value) => {
            tracing::warn!(request_id = %value, "audit request_id invalid; dropped from event");
            None
        }
    };
    Ok(repo::NewAuditLog {
        id: uuid::Uuid::new_v4(),
        created_at: chrono::Utc::now(),
        actor_type,
        actor_id: event.actor_id,
        actor_email: sanitize_optional_text(event.actor_email.as_deref(), 320),
        action,
        resource_type,
        resource_id: sanitize_optional_text(event.resource_id.as_deref(), 512),
        outcome,
        client_ip: sanitize_client_ip(event.client_ip.as_deref()),
        user_agent: sanitize_user_agent(&event.user_agent),
        request_id,
        detail: sanitize_audit_detail(event.detail.clone()),
    })
}

fn validate_audit_actor_type(raw: &str) -> Result<String, AppError> {
    let value = raw.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "admin" | "system" | "anonymous") {
        Ok(value)
    } else {
        Err(AppError::validation_field(
            "actor_type",
            "actor_type 只支持 admin/system/anonymous",
        ))
    }
}

fn validate_audit_outcome(raw: &str) -> Result<String, AppError> {
    let value = raw.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "success" | "failure" | "denied") {
        Ok(value)
    } else {
        Err(AppError::validation_field(
            "outcome",
            "outcome 只支持 success/failure/denied",
        ))
    }
}

/// 动作注册表格式校验：`{domain}.{resource}.{verb}`（允许 `.`/`_`/`-`，≤200）。
fn validate_audit_action(raw: &str) -> Result<String, AppError> {
    let value = raw.trim();
    let valid = !value.is_empty()
        && value.chars().count() <= 200
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if valid {
        Ok(value.to_string())
    } else {
        Err(AppError::validation_field(
            "action",
            "action 必须是 {domain}.{resource}.{verb} 格式（≤200，[a-z0-9._-]）",
        ))
    }
}

fn validate_audit_resource_type(raw: &str) -> Result<String, AppError> {
    let value = raw.trim();
    let valid = !value.is_empty()
        && value.chars().count() <= 100
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if valid {
        Ok(value.to_string())
    } else {
        Err(AppError::validation_field(
            "resource_type",
            "resource_type 长度必须在 1–100 之间（[a-z0-9._-]）",
        ))
    }
}

async fn insert_audit_once(state: &AppState, record: &repo::NewAuditLog) -> Result<(), AppError> {
    let pool = db(state)?;
    repo::insert_audit_log(pool, record).await
}

async fn write_audit_with_retry(state: &AppState, record: &repo::NewAuditLog, action: &str) {
    let mut last_error: Option<String> = None;
    for (attempt, delay_ms) in RETRY_DELAYS_MS.iter().enumerate() {
        if *delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
        }
        match insert_audit_once(state, record).await {
            Ok(()) => return,
            Err(error) => {
                if attempt + 1 < WRITE_ATTEMPTS {
                    tracing::warn!(
                        action = %action,
                        attempt = attempt + 1,
                        %error,
                        "audit write failed; retrying"
                    );
                }
                last_error = Some(error.to_string());
            }
        }
    }
    if let Some(error) = last_error {
        tracing::warn!(
            action = %action,
            attempts = WRITE_ATTEMPTS,
            error = %error,
            "audit write dropped after retries"
        );
    }
}

// ---------- S5.1 审计查询 ----------

/// `/api/admin/audit`：时间/操作者/动作/资源/结果过滤 + 分页（latest first）。
pub async fn list_audits(state: &AppState, query: &AuditListQuery) -> Result<AuditPage, AppError> {
    let pool = db(state)?;
    let (filters, limit, offset) = build_audit_filters(query)?;
    let (rows, total) = repo::list_audit_logs(pool, &filters, limit, offset).await?;
    Ok(AuditPage {
        items: rows.into_iter().map(to_audit_item).collect(),
        total,
        limit,
        offset,
    })
}

/// 审计筛选校验（列表/导出共用；默认最近 7 天）。
fn build_audit_filters(query: &AuditListQuery) -> Result<(AuditFilters, i64, i64), AppError> {
    let now = chrono::Utc::now();
    let from = query
        .from
        .unwrap_or(now - ChronoDuration::days(AUDIT_DEFAULT_WINDOW_DAYS));
    let to = query.to.unwrap_or(now);
    if from >= to {
        return Err(AppError::bad_request(
            "invalid_time_range",
            "from 必须早于 to",
        ));
    }
    let limit = query.limit.unwrap_or(AUDIT_DEFAULT_PAGE_LIMIT);
    if !(1..=AUDIT_MAX_PAGE_LIMIT).contains(&limit) {
        return Err(AppError::validation_field(
            "limit",
            format!("limit 必须在 1–{AUDIT_MAX_PAGE_LIMIT} 之间"),
        ));
    }
    let offset = query.offset.unwrap_or(0);
    if !(0..=100_000).contains(&offset) {
        return Err(AppError::validation_field(
            "offset",
            "offset 必须在 0–100000 之间",
        ));
    }
    let outcome = match query.outcome.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) => Some(validate_audit_outcome(value)?),
    };
    let action = match query.action.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) => Some(validate_audit_action(value)?),
    };
    let resource_type = match query.resource_type.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) => Some(validate_audit_resource_type(value)?),
    };
    let resource_id = match query.resource_id.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) if value.chars().count() > 512 => {
            return Err(AppError::validation_field(
                "resource_id",
                "resource_id 不能超过 512 个字符",
            ));
        }
        Some(value) => Some(value.to_string()),
    };
    let filters = AuditFilters {
        from,
        to,
        actor_id: query.actor_id,
        action,
        resource_type,
        resource_id,
        outcome,
    };
    Ok((filters, limit, offset))
}

fn to_audit_item(row: AuditLogRow) -> AuditItem {
    AuditItem {
        id: row.id,
        created_at: row.created_at,
        actor_type: row.actor_type,
        actor_id: row.actor_id,
        actor_email: row.actor_email,
        action: row.action,
        resource_type: row.resource_type,
        resource_id: row.resource_id,
        outcome: row.outcome,
        client_ip: row.client_ip,
        user_agent: row.user_agent,
        request_id: row.request_id,
        detail: sanitize_audit_detail(row.detail),
    }
}

// ---------- S5.1 CSV 导出（导出动作自身写审计；写失败不阻塞） ----------

/// 导出请求日志 CSV（筛选同 `/requests`；默认最近 7 天；>10 万行 400）。
pub async fn export_requests_csv(
    state: &AppState,
    query: &RequestListQuery,
    actor: &ExportActor,
) -> Result<CsvExport, AppError> {
    match export_requests_inner(state, query).await {
        Ok(export) => {
            record_export_audit(
                state,
                actor,
                "observability.request.exported",
                "request_log",
                "success",
                json!({"rows": export.row_count, "filters": export.summary}),
            );
            Ok(export)
        }
        Err(error) => {
            record_export_audit(
                state,
                actor,
                "observability.request.exported",
                "request_log",
                "failure",
                json!({"error_code": app_error_code(&error)}),
            );
            Err(error)
        }
    }
}

async fn export_requests_inner(
    state: &AppState,
    query: &RequestListQuery,
) -> Result<CsvExport, AppError> {
    let pool = db(state)?;
    let (filters, _, _) =
        build_request_filters(query, ChronoDuration::days(AUDIT_DEFAULT_WINDOW_DAYS))?;
    let rows = repo::export_request_logs(pool, &filters, MAX_EXPORT_ROWS).await?;
    Ok(CsvExport {
        filename: csv::requests_filename(filters.from, filters.to),
        body: csv::render_request_logs(&rows),
        row_count: rows.len(),
        summary: request_filters_summary(&filters),
    })
}

/// 导出审计 CSV（筛选同 `/audit`；默认最近 7 天；>10 万行 400）。
pub async fn export_audits_csv(
    state: &AppState,
    query: &AuditListQuery,
    actor: &ExportActor,
) -> Result<CsvExport, AppError> {
    match export_audits_inner(state, query).await {
        Ok(export) => {
            record_export_audit(
                state,
                actor,
                "observability.audit.exported",
                "audit_log",
                "success",
                json!({"rows": export.row_count, "filters": export.summary}),
            );
            Ok(export)
        }
        Err(error) => {
            record_export_audit(
                state,
                actor,
                "observability.audit.exported",
                "audit_log",
                "failure",
                json!({"error_code": app_error_code(&error)}),
            );
            Err(error)
        }
    }
}

async fn export_audits_inner(
    state: &AppState,
    query: &AuditListQuery,
) -> Result<CsvExport, AppError> {
    let pool = db(state)?;
    let (filters, _, _) = build_audit_filters(query)?;
    let rows = repo::export_audit_logs(pool, &filters, MAX_EXPORT_ROWS).await?;
    Ok(CsvExport {
        filename: csv::audit_filename(filters.from, filters.to),
        body: csv::render_audit_logs(&rows),
        row_count: rows.len(),
        summary: audit_filters_summary(&filters),
    })
}

fn record_export_audit(
    state: &AppState,
    actor: &ExportActor,
    action: &str,
    resource_type: &str,
    outcome: &str,
    detail: Value,
) {
    record_audit_detached(
        state,
        AuditEvent {
            actor_type: "admin".to_string(),
            actor_id: Some(actor.actor_id),
            actor_email: Some(actor.actor_email.clone()),
            action: action.to_string(),
            resource_type: resource_type.to_string(),
            resource_id: None,
            outcome: outcome.to_string(),
            client_ip: actor.client_ip.clone(),
            user_agent: actor.user_agent.clone(),
            request_id: actor.request_id.clone(),
            detail,
        },
    );
}

fn request_filters_summary(filters: &RequestFilters) -> Value {
    json!({
        "from": filters.from,
        "to": filters.to,
        "user_id": filters.user_id,
        "device_id": filters.device_id,
        "key_id": filters.key_id,
        "key_prefix": filters.key_prefix,
        "model": filters.model_alias,
        "provider": filters.provider,
        "status_min": filters.status_min,
        "status_max": filters.status_max,
        "trace_id": filters.trace_id,
    })
}

fn audit_filters_summary(filters: &AuditFilters) -> Value {
    json!({
        "from": filters.from,
        "to": filters.to,
        "actor_id": filters.actor_id,
        "action": filters.action,
        "resource_type": filters.resource_type,
        "resource_id": filters.resource_id,
        "outcome": filters.outcome,
    })
}

/// 稳定错误码（审计 detail 只记录 code，避免泄漏内部错误/SQL 文案）。
///
/// 供导出审计与 identity 登录/setup 显式审计复用。
pub(crate) fn app_error_code(error: &AppError) -> &'static str {
    match error {
        AppError::Validation { .. } => "validation_failed",
        AppError::Unauthorized { code, .. }
        | AppError::Forbidden { code, .. }
        | AppError::Conflict { code, .. }
        | AppError::BadRequest { code, .. }
        | AppError::NotFound { code, .. }
        | AppError::UnsupportedMediaType { code, .. }
        | AppError::Throttled { code, .. }
        | AppError::BadGateway { code, .. }
        | AppError::GatewayTimeout { code, .. } => code,
        AppError::TooManyAttempts { .. } => "too_many_attempts",
        AppError::PayloadTooLarge { .. } => "request_too_large",
        AppError::ModelNotFound { .. } => "model_not_found",
        AppError::CredentialUnavailable => "credential_unavailable",
        AppError::DatabaseUnavailable => "database_unavailable",
        AppError::NotImplemented(_) => "not_implemented",
        AppError::Internal => "internal_error",
    }
}

// ---------- S5.1 保留清理 ----------

/// 清理一次（测试/手动/后台 tick 共用）：错误只记日志，下轮重试。
pub async fn run_cleanup_once(state: &AppState) {
    if state.db.is_none() {
        return;
    }
    match cleanup_expired_request_logs(state).await {
        Ok(0) => {}
        Ok(deleted) => tracing::info!(deleted, "request_logs retention cleanup deleted rows"),
        Err(error) => tracing::error!(%error, "request_logs cleanup failed; retry next tick"),
    }
    match cleanup_expired_audit_logs(state).await {
        Ok(0) => {}
        Ok(deleted) => tracing::info!(deleted, "audit_logs retention cleanup deleted rows"),
        Err(error) => tracing::error!(%error, "audit_logs cleanup failed; retry next tick"),
    }
}

/// 后台清理任务：启动后立即执行一次，随后按 `log_cleanup_interval_secs` tick。
pub fn spawn_cleanup(state: AppState) {
    if state.db.is_none() {
        tracing::warn!("observability cleanup not started: database disabled");
        return;
    }
    let interval_secs = state.config.log_cleanup_interval_secs;
    let tick_state = state.clone();
    tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs);
        loop {
            run_cleanup_once(&tick_state).await;
            tokio::time::sleep(interval).await;
        }
    });
    tracing::info!(interval_secs, "observability cleanup task started");
}

async fn cleanup_expired_request_logs(state: &AppState) -> Result<u64, AppError> {
    let pool = db(state)?;
    // 运行期权威：app_settings（PATCH 后下一 tick 生效）；config 仅 DB 不可用时兜底。
    let retention_days =
        i64::from(crate::modules::settings::runtime::current().request_log_retention_days);
    let cutoff = chrono::Utc::now() - ChronoDuration::days(retention_days);
    let mut total = 0u64;
    for _ in 0..LOG_CLEANUP_MAX_BATCHES {
        let deleted =
            repo::delete_expired_request_logs_batch(pool, cutoff, LOG_CLEANUP_BATCH).await?;
        total += deleted;
        if deleted < LOG_CLEANUP_BATCH as u64 {
            break;
        }
    }
    Ok(total)
}

async fn cleanup_expired_audit_logs(state: &AppState) -> Result<u64, AppError> {
    let pool = db(state)?;
    let retention_days =
        i64::from(crate::modules::settings::runtime::current().audit_retention_days);
    let cutoff = chrono::Utc::now() - ChronoDuration::days(retention_days);
    let mut total = 0u64;
    for _ in 0..LOG_CLEANUP_MAX_BATCHES {
        let deleted =
            repo::delete_expired_audit_logs_batch(pool, cutoff, LOG_CLEANUP_BATCH).await?;
        total += deleted;
        if deleted < LOG_CLEANUP_BATCH as u64 {
            break;
        }
    }
    Ok(total)
}

/// 单测/集成测试用 401 事件（身份字段为空，字段值均在约束内）。
#[cfg(test)]
pub(crate) fn test_event(
    request_id: &str,
    created_at: chrono::DateTime<chrono::Utc>,
) -> RequestLogEvent {
    RequestLogEvent {
        request_id: request_id.to_string(),
        trace_id: Some(request_id.to_string()),
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        protocol: super::model::ProtocolKind::ChatCompletions,
        upstream_protocol: Some("openai_chat".to_string()),
        adapter: Some("chat_passthrough".to_string()),
        user_id: None,
        device_id: None,
        key_id: None,
        key_prefix: None,
        model_alias: Some("fast".to_string()),
        upstream_model: Some("deepseek-chat".to_string()),
        provider_id: None,
        provider: Some("TestProvider".to_string()),
        is_stream: false,
        fallback: false,
        attempt_count: 1,
        status_code: 401,
        latency_ms: 12,
        ttft_ms: None,
        response_size: None,
        error_code: Some("not_authenticated".to_string()),
        client_ip: Some("10.0.3.7".to_string()),
        user_agent: Some("curl/8.0".to_string()),
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::identity::model::SESSION_COOKIE;
    use crate::platform::{config::Config, crypto::Crypto};
    use axum::{
        Router,
        body::Body,
        extract::ConnectInfo,
        http::{HeaderMap, Method, Request, StatusCode},
    };
    use chrono::Utc;
    use http_body_util::BodyExt;
    use rust_decimal::Decimal;
    use serde_json::{Value, json};
    use std::net::SocketAddr;
    use std::{str::FromStr, sync::Arc, time::Instant};
    use tower::ServiceExt;
    use uuid::Uuid;

    fn test_state(db: Option<sqlx::PgPool>) -> AppState {
        let config = Config {
            bind: "127.0.0.1:0".parse().expect("test bind addr"),
            database_url: None,
            log_filter: "off".to_string(),
            run_migrations: false,
            cookie_secure: false,
            setup_token: None,
            usd_cny_rate: Decimal::from_str("7.2").expect("fx"),
            trust_proxy_headers: false,
            proxy_max_body_bytes: 8 * 1024 * 1024,
            proxy_expose_provider_header: false,
            usage_retention_days: 180,
            rollup_retention_days: 1825,
            rollup_interval_secs: 300,
            request_log_retention_days: 30,
            audit_retention_days: 180,
            log_cleanup_interval_secs: 3600,
        };
        AppState {
            config: Arc::new(config),
            db,
            crypto: Arc::new(Crypto::from_key_bytes([11u8; 32])),
            http: crate::platform::http::shared_client(),
            started_at: Instant::now(),
        }
    }

    #[test]
    fn prepare_record_sanitizes_and_normalizes() {
        let mut event = test_event("req-1", Utc::now());
        event.path = "/v1/chat/completions?stream=true&key=secret".to_string();
        event.method = "post".to_string();
        event.user_agent = Some("curl/8.0\r\nX-Injected: evil\u{0}".to_string());
        event.client_ip = Some("10.0.3.7:8080".to_string());
        event.error_code = Some("Bad Code".to_string());
        event.key_prefix = Some("not-a-key".to_string());
        event.ttft_ms = Some(50);
        event.attempt_count = 9;
        event.latency_ms = -5;
        event.response_size = Some(-1);
        event.status_code = 700;

        let record = prepare_record(&event).expect("prepare");
        assert_eq!(record.path, "/v1/chat/completions");
        assert_eq!(record.method, "POST");
        assert_eq!(record.user_agent, "curl/8.0 X-Injected: evil");
        assert_eq!(record.client_ip, None);
        assert_eq!(record.error_code.as_deref(), Some("gateway_error"));
        assert_eq!(record.key_prefix, None);
        assert_eq!(record.ttft_ms, None, "非流式请求不记录 ttft");
        assert_eq!(record.attempt_count, 5);
        assert_eq!(record.latency_ms, 0);
        assert_eq!(record.response_size, None);
        assert_eq!(record.status_code, None);
        assert_eq!(record.protocol, "chat_completions");
    }

    #[test]
    fn prepare_record_keeps_valid_fields() {
        let event = test_event("6a1c9e40-1f77-4a64-9d8e-0b9f2a5e6c11", Utc::now());
        let record = prepare_record(&event).expect("prepare");
        assert_eq!(
            record.trace_id.as_deref(),
            Some("6a1c9e40-1f77-4a64-9d8e-0b9f2a5e6c11")
        );
        assert_eq!(record.client_ip.as_deref(), Some("10.0.3.7"));
        assert_eq!(record.status_code, Some(401));
        assert_eq!(record.error_code.as_deref(), Some("not_authenticated"));
        assert_eq!(record.user_agent, "curl/8.0");
    }

    #[test]
    fn prepare_record_rejects_invalid_ids() {
        let mut event = test_event("bad request id", Utc::now());
        assert!(matches!(
            prepare_record(&event),
            Err(AppError::Validation { .. })
        ));

        event.request_id = "req-valid".to_string();
        event.trace_id = Some("bad\ntrace".to_string());
        assert!(matches!(
            prepare_record(&event),
            Err(AppError::Validation { .. })
        ));
    }

    #[tokio::test]
    async fn flush_returns_true_once_in_flight_drains() {
        // 全局 in-flight 计数可能被同模块并行测试占用，给足排空时间。
        assert!(flush(Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn record_request_log_without_db_fails_closed() {
        let state = test_state(None);
        let event = test_event("req-no-db", Utc::now());
        assert!(matches!(
            record_request_log(&state, event).await,
            Err(AppError::DatabaseUnavailable)
        ));
    }

    #[tokio::test]
    async fn detached_invalid_event_is_dropped_without_write_attempt() {
        let state = test_state(None);
        let mut event = test_event("valid-request-id", Utc::now());
        event.request_id = "has space".to_string();
        record_request_log_detached(&state, event);
        // 非法事件在 spawn 前丢弃（不产生 DB/重试任务）；全局 in-flight 可能被
        // 同模块并行测试占用，给足排空时间。
        assert!(flush(Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn detached_write_drains_through_flush() {
        let state = test_state(None);
        record_request_log_detached(&state, test_event("req-detached", Utc::now()));
        // DB 关闭时 3 次尝试共约 1.2s 后放弃；flush 应等到 in-flight 归零。
        assert!(flush(Duration::from_secs(5)).await);
    }

    async fn test_state_db() -> Option<AppState> {
        let url = std::env::var("GW_TEST_DATABASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())?;
        let pool = sqlx::PgPool::connect(&url)
            .await
            .expect("connect GW_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        // 共享测试库：不做 TRUNCATE（其他模块 gated 测试可能并发使用）；
        // 所有用例使用随机 request_id，断言只针对本用例数据。
        Some(test_state(Some(pool)))
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 request_logs；用例使用随机 id，不 TRUNCATE）"]
    async fn record_request_log_is_idempotent_and_maps_fields() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let request_id = format!("req-log-{}", Uuid::new_v4());
        let mut event = test_event(&request_id, Utc::now());
        // 脏输入：写入前应被 sanitizer 归一。
        event.path = "/v1/chat/completions?secret=1".to_string();
        event.method = "post".to_string();
        event.user_agent = Some("curl/8.0\r\nInjected".to_string());
        event.client_ip = Some("10.0.3.7:1234".to_string());
        event.error_code = Some("Not Valid".to_string());
        event.key_prefix = Some("sk-gw-abcd".to_string());
        event.model_alias = Some("fast".to_string());
        event.status_code = 200;
        record_request_log(&state, event.clone())
            .await
            .expect("first insert");
        record_request_log(&state, event)
            .await
            .expect("duplicate insert");

        let pool = state.db.as_ref().expect("pool");
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM request_logs WHERE request_id = $1")
                .bind(&request_id)
                .fetch_one(pool)
                .await
                .expect("count rows");
        assert_eq!(count, 1);

        let row: (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT method, path, user_agent, client_ip, key_prefix, error_code \
                 FROM request_logs WHERE request_id = $1",
        )
        .bind(&request_id)
        .fetch_one(pool)
        .await
        .expect("fetch row");
        assert_eq!(row.0, "POST");
        assert_eq!(row.1, "/v1/chat/completions");
        assert_eq!(row.2, "curl/8.0 Injected");
        assert_eq!(row.3, None);
        assert_eq!(row.4.as_deref(), Some("sk-gw-abcd"));
        assert_eq!(row.5.as_deref(), Some("gateway_error"));
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 request_logs；用例使用随机 id，不 TRUNCATE）"]
    async fn unauthenticated_log_keeps_identity_null_and_stream_fields() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let request_id = format!("req-log-401-{}", Uuid::new_v4());
        let mut event = test_event(&request_id, Utc::now());
        event.is_stream = true;
        event.ttft_ms = Some(380);
        event.response_size = Some(15_320);
        event.latency_ms = 1_840;
        record_request_log(&state, event).await.expect("insert");

        #[derive(sqlx::FromRow)]
        struct StreamLogRow {
            user_id: Option<Uuid>,
            device_id: Option<Uuid>,
            key_id: Option<Uuid>,
            is_stream: bool,
            ttft_ms: Option<i32>,
            response_size: Option<i64>,
            latency_ms: i32,
        }

        let pool = state.db.as_ref().expect("pool");
        let row: StreamLogRow = sqlx::query_as(
            "SELECT user_id, device_id, key_id, is_stream, ttft_ms, response_size, latency_ms \
             FROM request_logs WHERE request_id = $1",
        )
        .bind(&request_id)
        .fetch_one(pool)
        .await
        .expect("fetch row");
        assert_eq!((row.user_id, row.device_id, row.key_id), (None, None, None));
        assert!(row.is_stream);
        assert_eq!(row.ttft_ms, Some(380));
        assert_eq!(row.response_size, Some(15_320));
        assert_eq!(row.latency_ms, 1_840);
    }

    // ---------- S5.1 单元测试：审计事件准备 ----------

    fn audit_event(
        actor_id: Option<Uuid>,
        action: &str,
        resource_type: &str,
        outcome: &str,
        detail: Value,
    ) -> AuditEvent {
        AuditEvent {
            actor_type: "admin".to_string(),
            actor_id,
            actor_email: Some("audit-admin@corp.example.com".to_string()),
            action: action.to_string(),
            resource_type: resource_type.to_string(),
            resource_id: Some("r-1".to_string()),
            outcome: outcome.to_string(),
            client_ip: Some("10.0.3.7".to_string()),
            user_agent: "curl/8.0".to_string(),
            request_id: Some("req-audit-1".to_string()),
            detail,
        }
    }

    #[test]
    fn prepare_audit_record_validates_and_sanitizes() {
        let mut event = audit_event(
            Some(Uuid::new_v4()),
            "access.key.revoked",
            "virtual_key",
            "Success",
            json!({"password": "x", "before": {"status": "active"}, "rows": 1}),
        );
        event.client_ip = Some("10.0.3.7:8080".to_string());
        event.user_agent = "a".repeat(600);
        let record = prepare_audit_record(&event).expect("prepare audit");
        assert_eq!(record.actor_type, "admin");
        assert_eq!(record.outcome, "success");
        assert_eq!(record.action, "access.key.revoked");
        assert_eq!(record.client_ip, None);
        assert_eq!(record.user_agent.chars().count(), 512);
        assert!(record.detail.get("password").is_none());
        assert_eq!(record.detail["before"]["status"], "active");

        // 非法 outcome/action/resource_type 一律校验失败。
        for (action, resource_type, outcome) in [
            ("access.key.revoked", "virtual_key", "weird"),
            ("Bad Action", "virtual_key", "success"),
            ("access.key.revoked", "", "success"),
            ("access.key.revoked", "virtual key", "success"),
        ] {
            let event = audit_event(None, action, resource_type, outcome, json!({}));
            assert!(
                matches!(
                    prepare_audit_record(&event),
                    Err(AppError::Validation { .. })
                ),
                "expected validation error for {action}/{resource_type}/{outcome}"
            );
        }
        // 非法 request_id 丢弃但不拒绝整条事件。
        let mut event = audit_event(None, "system.task.ran", "system", "success", json!({}));
        event.request_id = Some("bad request id".to_string());
        let record = prepare_audit_record(&event).expect("prepare audit");
        assert_eq!(record.request_id, None);
    }

    // ---------- S5.1 gated helpers ----------

    async fn seed_admin_user(state: &AppState) -> (Uuid, String) {
        let pool = state.db.as_ref().expect("pool");
        let admin_id = Uuid::new_v4();
        let email = format!("audit-s51-{admin_id}@corp.example.com");
        sqlx::query(
            "INSERT INTO admin_users (id, email, password_hash) VALUES ($1, $2, 'not-a-real-hash')",
        )
        .bind(admin_id)
        .bind(&email)
        .execute(pool)
        .await
        .expect("insert admin user");
        (admin_id, email)
    }

    /// 直接插入 AdminSession（不经过 identity 登录流程），返回 Cookie 头值。
    async fn seed_admin_session(state: &AppState, admin_id: Uuid) -> String {
        let pool = state.db.as_ref().expect("pool");
        let token = format!("s51-token-{admin_id}");
        let token_hash = crate::modules::identity::security::sha256_hex(&token);
        sqlx::query(
            "INSERT INTO admin_sessions (id, user_id, csrf_token, token_hash, expires_at) \
             VALUES ($1, $2, 'csrf-token', $3, now() + interval '1 hour')",
        )
        .bind(Uuid::new_v4())
        .bind(admin_id)
        .bind(token_hash)
        .execute(pool)
        .await
        .expect("insert admin session");
        format!("{SESSION_COOKIE}={token}")
    }

    async fn cleanup_admin(state: &AppState, admin_id: Uuid) {
        let Some(pool) = state.db.as_ref() else {
            return;
        };
        let _ = sqlx::query("DELETE FROM audit_logs WHERE actor_id = $1")
            .bind(admin_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1")
            .bind(admin_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM admin_users WHERE id = $1")
            .bind(admin_id)
            .execute(pool)
            .await;
    }

    async fn http_call(
        app: &Router,
        method: Method,
        uri: &str,
        headers: &[(&str, String)],
    ) -> (StatusCode, HeaderMap, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, value);
        }
        let mut request = builder.body(Body::empty()).expect("build request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
        let response = app.clone().oneshot(request).await.expect("request");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }

    // ---------- S5.1 gated：审计写入/过滤/分页/脱敏 ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 audit_logs；用例使用随机 id）"]
    async fn audit_write_and_query_filters_sanitize_ua() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (admin_id, email) = seed_admin_user(&state).await;
        let action = "s51.audit.created";
        let resource_type = "s51_audit_resource";
        let since = Utc::now() - chrono::Duration::hours(1);
        let until = Utc::now() + chrono::Duration::hours(1);

        let mut created = audit_event(
            Some(admin_id),
            action,
            resource_type,
            "success",
            json!({"password": "hunter2", "token": "t-1", "daily_limit": {"before": "5.000000"}}),
        );
        created.actor_email = Some(email.clone());
        record_audit(&state, created).await.expect("record created");

        let mut failed = audit_event(
            Some(admin_id),
            action,
            resource_type,
            "failure",
            json!({"error_code": "validation_failed"}),
        );
        failed.actor_email = Some(email.clone());
        record_audit(&state, failed).await.expect("record failed");

        let mut denied = audit_event(
            Some(admin_id),
            "s51.audit.deleted",
            resource_type,
            "denied",
            json!({"password": "x"}),
        );
        denied.actor_email = Some(email.clone());
        denied.client_ip = Some("10.0.3.7:8080".to_string());
        denied.user_agent = "a".repeat(600);
        record_audit(&state, denied).await.expect("record denied");

        let mut detached = audit_event(
            Some(admin_id),
            "s51.audit.detached",
            resource_type,
            "success",
            json!({}),
        );
        detached.actor_email = Some(email.clone());
        record_audit_detached(&state, detached);
        assert!(flush(Duration::from_secs(5)).await);

        let base = AuditListQuery {
            from: Some(since),
            to: Some(until),
            actor_id: Some(admin_id),
            action: None,
            resource_type: None,
            resource_id: None,
            outcome: None,
            limit: Some(10),
            offset: Some(0),
        };
        let page = list_audits(&state, &base).await.expect("list audit");
        assert_eq!(page.total, 4);
        assert_eq!(page.items.len(), 4);
        // latest first（denied/detached 最后写入，至少首条时间不早于末条）。
        assert!(page.items[0].created_at >= page.items[3].created_at);
        // detail 已脱敏：任何层级无 password/token，保留非敏感摘要。
        for item in &page.items {
            assert!(item.detail.get("password").is_none());
            assert!(item.detail.get("token").is_none());
        }
        assert!(
            page.items
                .iter()
                .any(|item| item.detail["daily_limit"]["before"] == "5.000000")
        );

        // action 过滤。
        let by_action = AuditListQuery {
            action: Some(action.to_string()),
            ..base.clone()
        };
        let page = list_audits(&state, &by_action).await.expect("by action");
        assert_eq!(page.total, 2);
        // outcome 过滤。
        let by_outcome = AuditListQuery {
            outcome: Some("failure".to_string()),
            ..base.clone()
        };
        assert_eq!(
            list_audits(&state, &by_outcome)
                .await
                .expect("by outcome")
                .total,
            1
        );
        // resource_type 过滤。
        let by_resource = AuditListQuery {
            resource_type: Some(resource_type.to_string()),
            ..base.clone()
        };
        assert_eq!(
            list_audits(&state, &by_resource)
                .await
                .expect("by resource")
                .total,
            4
        );
        // 分页 + 排序。
        let paged = AuditListQuery {
            limit: Some(2),
            offset: Some(1),
            ..base.clone()
        };
        let page = list_audits(&state, &paged).await.expect("paged");
        assert_eq!(page.total, 4);
        assert_eq!(page.items.len(), 2);
        assert_eq!((page.limit, page.offset), (2, 1));

        // limit > 500 / 非法 outcome → 400。
        let too_many = AuditListQuery {
            limit: Some(501),
            ..base.clone()
        };
        assert!(matches!(
            list_audits(&state, &too_many).await,
            Err(AppError::Validation { .. })
        ));
        let bad_outcome = AuditListQuery {
            outcome: Some("bogus".to_string()),
            ..base.clone()
        };
        assert!(matches!(
            list_audits(&state, &bad_outcome).await,
            Err(AppError::Validation { .. })
        ));

        // 落库侧：UA 截断 512、非法 client_ip 归一为 NULL。
        let pool = state.db.as_ref().expect("pool");
        let (ua_len, client_ip): (i32, Option<String>) = sqlx::query_as(
            "SELECT char_length(user_agent), client_ip FROM audit_logs \
             WHERE actor_id = $1 AND action = 's51.audit.deleted'",
        )
        .bind(admin_id)
        .fetch_one(pool)
        .await
        .expect("ua row");
        assert_eq!(ua_len, 512);
        assert_eq!(client_ip, None);

        // 非法事件同步接口返回校验错误（不落库）。
        let invalid = audit_event(
            Some(admin_id),
            "s51.audit.created",
            resource_type,
            "bogus",
            json!({}),
        );
        assert!(matches!(
            record_audit(&state, invalid).await,
            Err(AppError::Validation { .. })
        ));

        cleanup_admin(&state, admin_id).await;
    }

    // ---------- S5.1 gated：保留清理只删过期 ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 request_logs/audit_logs；用例使用随机 id）"]
    async fn cleanup_only_deletes_expired_request_and_audit_rows() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let pool = state.db.as_ref().expect("pool");
        let suffix = Uuid::new_v4();
        let old_request = format!("s51-clean-old-{suffix}");
        let new_request = format!("s51-clean-new-{suffix}");
        let old_audit_action = "s51.cleanup.expired";
        let new_audit_action = "s51.cleanup.recent";
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO request_logs (id, request_id, method, path, created_at) \
             VALUES (gen_random_uuid(), $1, 'GET', '/v1/models', $2), \
                    (gen_random_uuid(), $3, 'GET', '/v1/models', $4)",
        )
        .bind(&old_request)
        .bind(now - chrono::Duration::days(40))
        .bind(&new_request)
        .bind(now - chrono::Duration::days(1))
        .execute(pool)
        .await
        .expect("seed request logs");
        sqlx::query(
            "INSERT INTO audit_logs (id, created_at, actor_type, action, resource_type, outcome) \
             VALUES (gen_random_uuid(), $1, 'system', $2, 'system', 'success'), \
                    (gen_random_uuid(), $3, 'system', $4, 'system', 'success')",
        )
        .bind(now - chrono::Duration::days(200))
        .bind(old_audit_action)
        .bind(now - chrono::Duration::days(1))
        .bind(new_audit_action)
        .execute(pool)
        .await
        .expect("seed audit logs");

        run_cleanup_once(&state).await;

        let old_request_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM request_logs WHERE request_id = $1")
                .bind(&old_request)
                .fetch_one(pool)
                .await
                .expect("count old request");
        assert_eq!(old_request_left, 0, "expired request log must be deleted");
        let new_request_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM request_logs WHERE request_id = $1")
                .bind(&new_request)
                .fetch_one(pool)
                .await
                .expect("count new request");
        assert_eq!(new_request_left, 1, "recent request log must be kept");
        let old_audit_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = $1")
                .bind(old_audit_action)
                .fetch_one(pool)
                .await
                .expect("count old audit");
        assert_eq!(old_audit_left, 0, "expired audit log must be deleted");
        let new_audit_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = $1")
                .bind(new_audit_action)
                .fetch_one(pool)
                .await
                .expect("count new audit");
        assert_eq!(new_audit_left, 1, "recent audit log must be kept");

        // 清理测试数据。
        let _ = sqlx::query("DELETE FROM request_logs WHERE request_id = ANY($1)")
            .bind(vec![old_request, new_request])
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM audit_logs WHERE action = ANY($1)")
            .bind(vec![old_audit_action, new_audit_action])
            .execute(pool)
            .await;
    }

    // ---------- S5.1 gated：导出超限（小上限注入） ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 request_logs/audit_logs；用例使用随机 id）"]
    async fn export_too_large_returns_400_code() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let pool = state.db.as_ref().expect("pool");
        let suffix = Uuid::new_v4();
        let now = Utc::now();
        for index in 0..3 {
            let request_id = format!("s51-export-{suffix}-{index}");
            let event = test_event(&request_id, now);
            record_request_log(&state, event)
                .await
                .expect("seed request log");
        }
        let (request_filters, _, _) = build_request_filters(
            &RequestListQuery {
                from: Some(now - chrono::Duration::hours(1)),
                to: Some(now + chrono::Duration::hours(1)),
                limit: Some(50),
                ..Default::default()
            },
            ChronoDuration::hours(24),
        )
        .expect("filters");
        let error = repo::export_request_logs(pool, &request_filters, 2)
            .await
            .expect_err("must exceed small max");
        assert!(
            matches!(
                error,
                AppError::BadRequest {
                    code: "export_too_large",
                    ..
                }
            ),
            "expected export_too_large, got {error}"
        );
        let rows = repo::export_request_logs(pool, &request_filters, 3)
            .await
            .expect("export under limit");
        assert!(rows.len() >= 3);

        // 审计导出同样受上限约束。
        let (admin_id, email) = seed_admin_user(&state).await;
        for index in 0..2 {
            let mut event = audit_event(
                Some(admin_id),
                "s51.export.exported",
                "system",
                "success",
                json!({"rows": index}),
            );
            event.actor_email = Some(email.clone());
            record_audit(&state, event).await.expect("seed audit");
        }
        let (audit_filters, _, _) = build_audit_filters(&AuditListQuery {
            from: Some(now - chrono::Duration::hours(1)),
            to: Some(now + chrono::Duration::hours(1)),
            actor_id: Some(admin_id),
            ..Default::default()
        })
        .expect("audit filters");
        let error = repo::export_audit_logs(pool, &audit_filters, 1)
            .await
            .expect_err("audit export must exceed small max");
        assert!(matches!(
            error,
            AppError::BadRequest {
                code: "export_too_large",
                ..
            }
        ));
        assert_eq!(
            repo::export_audit_logs(pool, &audit_filters, 2)
                .await
                .expect("audit export under limit")
                .len(),
            2
        );

        let _ = sqlx::query("DELETE FROM request_logs WHERE request_id LIKE $1")
            .bind(format!("s51-export-{suffix}-%"))
            .execute(pool)
            .await;
        cleanup_admin(&state, admin_id).await;
    }

    // ---------- S5.1 gated HTTP：audit 不再 501 + no-store + CSV 导出/导出审计 ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（HTTP 路由 + session/CSV；用例使用随机 id）"]
    async fn audit_and_export_http_contract() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let app = crate::router::app(state.clone());
        let pool = state.db.as_ref().expect("pool").clone();
        let (admin_id, email) = seed_admin_user(&state).await;
        let cookie = seed_admin_session(&state, admin_id).await;
        let auth = [("cookie", cookie)];
        let suffix = Uuid::new_v4();
        let now = Utc::now();

        // 种子请求日志：含逗号/公式触发字符，用于 CSV 转义断言。
        for (index, model_alias) in ["fast,smart", "=SUM(A1)"].iter().enumerate() {
            let request_id = format!("s51-http-{suffix}-{index}");
            let mut event = test_event(&request_id, now);
            event.model_alias = Some(model_alias.to_string());
            event.user_agent = Some("=HYPERLINK(\"http://evil\")".to_string());
            record_request_log(&state, event)
                .await
                .expect("seed request log");
        }
        // 一条审计行（用于列表断言）。
        let mut audit = audit_event(
            Some(admin_id),
            "s51.audit.http_created",
            "s51_audit_resource",
            "success",
            json!({"password": "x", "rows": 1}),
        );
        audit.actor_email = Some(email.clone());
        record_audit(&state, audit).await.expect("seed audit");

        // 未登录：401 + problem+json（audit 不再 501）。
        for uri in [
            "/api/admin/audit",
            "/api/admin/requests/export",
            "/api/admin/audit/export",
        ] {
            let (status, headers, _) = http_call(&app, Method::GET, uri, &[]).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "uri={uri}");
            assert_eq!(
                header_str(&headers, "content-type"),
                Some("application/problem+json")
            );
        }

        // 审计列表：200 + no-store + 过滤生效 + detail 已脱敏。
        let uri = format!("/api/admin/audit?actor_id={admin_id}&limit=10");
        let (status, headers, body) = http_call(&app, Method::GET, &uri, &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
        let page: Value = serde_json::from_str(&body).expect("audit json");
        assert_eq!(page["total"], 1);
        assert!(page["items"][0].get("id").is_some());
        assert!(page["items"][0]["detail"].get("password").is_none());

        // 审计参数校验：limit>500 → 400。
        let (status, _, _) =
            http_call(&app, Method::GET, "/api/admin/audit?limit=501", &auth).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // requests CSV 导出：200 + 头/转义/公式防护 + no-store。
        let (status, headers, csv_body) =
            http_call(&app, Method::GET, "/api/admin/requests/export", &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            header_str(&headers, "content-type"),
            Some("text/csv; charset=utf-8")
        );
        assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
        let disposition = header_str(&headers, "content-disposition").expect("disposition");
        assert!(disposition.starts_with("attachment; filename=\"requests-"));
        assert!(disposition.ends_with(".csv\""));
        assert!(csv_body.starts_with("id,request_id,trace_id,created_at,method,path,protocol,"));
        assert!(csv_body.contains(&format!("s51-http-{suffix}-0")));
        assert!(csv_body.contains("\"fast,smart\""));
        assert!(csv_body.contains("'=SUM(A1)"));
        assert!(csv_body.contains("'=HYPERLINK"));

        // 导出成功动作写审计（detached；flush 后落表）。
        assert!(flush(Duration::from_secs(5)).await);
        let request_export_audits: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_logs \
             WHERE actor_id = $1 AND action = 'observability.request.exported' AND outcome = 'success'",
        )
        .bind(admin_id)
        .fetch_one(&pool)
        .await
        .expect("count request export audit");
        assert_eq!(request_export_audits, 1);
        let export_detail: Value = sqlx::query_scalar(
            "SELECT detail FROM audit_logs \
             WHERE actor_id = $1 AND action = 'observability.request.exported' \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(admin_id)
        .fetch_one(&pool)
        .await
        .expect("request export detail");
        assert!(export_detail["rows"].as_u64().is_some_and(|rows| rows >= 2));
        assert!(export_detail["filters"].is_object());

        // audit CSV 导出：200 + 列表头 + 导出动作审计。
        let (status, headers, audit_csv) =
            http_call(&app, Method::GET, "/api/admin/audit/export", &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            header_str(&headers, "content-type"),
            Some("text/csv; charset=utf-8")
        );
        assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
        assert!(audit_csv.starts_with("id,created_at,actor_type,actor_id,actor_email,action,"));
        assert!(flush(Duration::from_secs(5)).await);
        let audit_export_audits: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_logs \
             WHERE actor_id = $1 AND action = 'observability.audit.exported' AND outcome = 'success'",
        )
        .bind(admin_id)
        .fetch_one(&pool)
        .await
        .expect("count audit export audit");
        assert_eq!(audit_export_audits, 1);

        // 清理测试数据。
        let _ = sqlx::query("DELETE FROM request_logs WHERE request_id LIKE $1")
            .bind(format!("s51-http-{suffix}-%"))
            .execute(&pool)
            .await;
        cleanup_admin(&state, admin_id).await;
    }
}
