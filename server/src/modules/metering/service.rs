//! metering 用例：用量写入（同步/旁路 + 优雅退出 flush）、预算读数、管理查询、日结/维护。
//!
//! 设计见 `设计文档（暂未开源）` §3/§4/§8 与
//! `设计文档（暂未开源）`：
//! - `record_usage`：单次幂等写入（测试/需确认时 await）；
//! - `record_usage_detached`：spawn + 有界重试（立即/200ms/1s，共 3 次）+ in-flight 计数，
//!   失败仅 `tracing::warn!`，**不影响主请求**；
//! - `flush`：main 优雅退出等待 in-flight 归零（≤timeout）；
//! - `today_spend`：Asia/Shanghai 当日 CNY 花费，失败关闭（DB 故障 → 503）；
//! - S5 查询：`overview`/`usage`；S5.1 已结账日读 rollup、其余读明细并在内存合并；
//! - S5.1 日结/维护：`finalize_day`/`finalize_due_days`/`run_maintenance_once`/`spawn_maintenance`
//!   （先日结后清理；明细仅清 finalized 过期日；错误日志、下轮重试）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::platform::{error::AppError, state::AppState};

use super::{
    cost,
    model::{CURRENCY_CNY, PRICE_CURRENCY_CNY, PRICE_CURRENCY_USD, UsageEvent},
    repo,
};

/// Asia/Shanghai 固定 UTC+08:00（无夏令时；与 policy 日界同口径，不引入 chrono-tz）。
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 3600;
/// 旁路写入总尝试次数（立即/200ms/1s）。
const WRITE_ATTEMPTS: usize = 3;
const RETRY_DELAYS_MS: [u64; WRITE_ATTEMPTS] = [0, 200, 1_000];

/// 已 spawn 但未落库的用例数（flush 等待用）。
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// in-flight 计数守卫：创建即 +1，drop（任务结束/失败）即 -1。
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
///
/// `request_id` 冲突返回 Ok（重复写入由唯一索引吸收）；
/// DB 不可用 → [`AppError::DatabaseUnavailable`]（失败关闭）。
pub async fn record_usage(state: &AppState, event: UsageEvent) -> Result<(), AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(AppError::database_unavailable)?;
    let record = prepare_record(state, &event)?;
    repo::insert_usage(pool, &record).await
}

/// proxy 默认入口：fire-and-forget + 有界重试 + in-flight 计数；失败只记日志。
pub fn record_usage_detached(state: &AppState, event: UsageEvent) {
    let state = state.clone();
    let request_id = event.request_id.clone();
    // 在 spawn 前 +1，避免调用方立刻 flush 时漏掉尚未被调度的任务。
    let guard = InFlightGuard::acquire();
    tokio::spawn(async move {
        let _guard = guard;
        write_with_retry(&state, &event, &request_id).await;
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

/// Asia/Shanghai 当日已用成本（CNY）；失败关闭，proxy 必须 503 而非按 0 放行。
pub async fn today_spend(state: &AppState, user_id: Uuid) -> Result<Decimal, AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(AppError::database_unavailable)?;
    let (start, end) = shanghai_day_bounds(Utc::now());
    repo::sum_user_cost(pool, user_id, start, end).await
}

/// 当日 `[start, end)`（UTC 瞬时）：Asia/Shanghai 00:00 起算。
fn shanghai_day_bounds(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let offset = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS).expect("valid +08:00 offset");
    let local_now = now.with_timezone(&offset);
    let start_local = local_now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("valid local midnight");
    let start = offset
        .from_local_datetime(&start_local)
        .single()
        .expect("unambiguous +08:00 local time")
        .with_timezone(&Utc);
    (start, start + chrono::Duration::days(1))
}

/// 事件 → `usage_records` 行：成本计算 + 约束兜底归一化。
fn prepare_record(state: &AppState, event: &UsageEvent) -> Result<repo::NewUsageRecord, AppError> {
    // S4 账本币种恒 CNY（DB CHECK）；事件里的 currency 仅用于契约完整性。
    if !event.currency.trim().is_empty()
        && !event.currency.trim().eq_ignore_ascii_case(CURRENCY_CNY)
    {
        tracing::warn!(
            request_id = %event.request_id,
            currency = %event.currency,
            "usage_records.currency is fixed to CNY; ignoring event currency"
        );
    }

    let price_currency = normalize_price_currency(event.price_currency.as_deref());
    // fx 快照只在 USD 价目时写入（DB CHECK fx_rate > 0 或 NULL）。
    let fx_rate = (price_currency == PRICE_CURRENCY_USD).then_some(state.config.usd_cny_rate);
    let price_basis = cost::PriceBasis {
        input: event.price_input,
        output: event.price_output,
        currency: price_currency.to_string(),
    };
    let input_tokens = clamp_tokens(event.input_tokens);
    let output_tokens = clamp_tokens(event.output_tokens);
    let request_cost = cost::compute_cost(
        i64::from(input_tokens),
        i64::from(output_tokens),
        &price_basis,
        state.config.usd_cny_rate,
    )
    .map_err(|error| AppError::internal(format!("compute usage cost: {error}")))?;

    // 约束兜底：ttft 仅流式、http_status 100–599、attempt_count 1–5、非负延迟。
    let ttft_ms = if event.is_stream {
        event
            .ttft_ms
            .filter(|value| *value >= 0)
            .map(|value| value.min(i64::from(i32::MAX)) as i32)
    } else {
        None
    };
    let http_status = (100..=599)
        .contains(&event.http_status)
        .then_some(event.http_status);
    let attempt_count = event.attempt_count.clamp(1, 5);
    let latency_ms = event.latency_ms.clamp(0, i64::from(i32::MAX)) as i32;

    Ok(repo::NewUsageRecord {
        id: Uuid::new_v4(),
        request_id: event.request_id.clone(),
        user_id: event.user_id,
        device_id: event.device_id,
        key_id: event.key_id,
        model_alias: event.model_alias.clone(),
        deployment_id: event.deployment_id,
        source: event.source.as_str(),
        resolved_upstream_model: event.resolved_upstream_model.clone().unwrap_or_default(),
        provider_id: event.provider_id,
        provider_name: event.provider_name.clone().unwrap_or_default(),
        input_tokens,
        output_tokens,
        usage_source: event.usage_source.as_str(),
        cost: request_cost,
        currency: CURRENCY_CNY,
        unit_price_input: event.price_input,
        unit_price_output: event.price_output,
        price_currency,
        fx_rate,
        latency_ms,
        ttft_ms,
        is_stream: event.is_stream,
        fallback: event.fallback,
        attempt_count,
        status: event.status.as_str(),
        http_status,
        error_code: event.error_code.clone().filter(|code| !code.is_empty()),
        created_at: event.created_at,
    })
}

/// 价目币种归一：None/空串视为 CNY（无价目）；未知值按 CNY 记账并告警（不丢台账）。
fn normalize_price_currency(raw: Option<&str>) -> &'static str {
    let Some(raw) = raw else {
        return PRICE_CURRENCY_CNY;
    };
    match raw.trim().to_ascii_uppercase().as_str() {
        "" | PRICE_CURRENCY_CNY => PRICE_CURRENCY_CNY,
        PRICE_CURRENCY_USD => PRICE_CURRENCY_USD,
        _ => {
            tracing::warn!(price_currency = %raw, "unknown price currency; defaulting to CNY");
            PRICE_CURRENCY_CNY
        }
    }
}

/// token 归一：None→0；负值→0；超过 INTEGER 上限（0007 列为 INTEGER）按上限截断。
fn clamp_tokens(tokens: Option<i64>) -> i32 {
    tokens.unwrap_or(0).clamp(0, i64::from(i32::MAX)) as i32
}

/// 旁路写入：按 `RETRY_DELAYS_MS` 重试，全部失败仅告警。
async fn write_with_retry(state: &AppState, event: &UsageEvent, request_id: &str) {
    let mut last_error: Option<String> = None;
    for (attempt, delay_ms) in RETRY_DELAYS_MS.iter().enumerate() {
        if *delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
        }
        match record_usage(state, event.clone()).await {
            Ok(()) => return,
            Err(error) => {
                if attempt + 1 < WRITE_ATTEMPTS {
                    tracing::warn!(
                        request_id = %request_id,
                        attempt = attempt + 1,
                        %error,
                        "usage write failed; retrying"
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
            "usage write dropped after retries"
        );
    }
}

// ---------- 公共 DB 入口 ----------

fn db(state: &AppState) -> Result<&sqlx::PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
}

// ---------- S5 管理查询（S5.1：已结账日读 rollup，其余读明细） ----------

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono::{Duration as ChronoDuration, NaiveDate};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use super::model::{
    Metrics, OverviewQuery, OverviewResponse, TodayMetrics, TrendDay, UsageItem, UsageQuery,
    UsageResponse,
};
use super::repo::{AggregateFilters, AggregateMetrics, GroupAggregate};

pub const OVERVIEW_TIMEZONE: &str = "Asia/Shanghai";
/// 读路径事务：同一快照内合并明细与 rollup，保证 totals 与分页行一致。
const READ_TX_SQL: &str = "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY";
/// 每 tick 最多日结天数（避免长事务；启动 backfill 由多轮 tick 收敛）。
const FINALIZE_MAX_DAYS_PER_TICK: usize = 7;
/// 明细保留清理：每批行数与每 tick 最多批数（避免长锁/长 tick）。
const DETAIL_CLEANUP_BATCH: i64 = 10_000;
const DETAIL_CLEANUP_MAX_BATCHES: usize = 10;

async fn begin_read_tx(pool: &PgPool) -> Result<Transaction<'static, Postgres>, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query(READ_TX_SQL).execute(&mut *tx).await?;
    Ok(tx)
}

/// `/api/admin/overview`：今日 + 最近 days 天趋势 + 窗口合计。
///
/// S5.1：已结账日（`usage_rollup_days.finalized=true` 且 `< today`）读 rollup，
/// 其余日（含今天）读 `usage_records`；合并后趋势/合计与明细口径一致。
pub async fn overview(
    state: &AppState,
    query: &OverviewQuery,
) -> Result<OverviewResponse, AppError> {
    overview_scoped(state, query, None).await
}

/// 客户端只读用量：按认证用户过滤，其余与管理台 overview 完全同口径。
pub async fn client_overview(
    state: &AppState,
    user_id: Uuid,
    days: i64,
) -> Result<OverviewResponse, AppError> {
    overview_scoped(state, &OverviewQuery { days: Some(days) }, Some(user_id)).await
}

async fn overview_scoped(
    state: &AppState,
    query: &OverviewQuery,
    user_id: Option<Uuid>,
) -> Result<OverviewResponse, AppError> {
    let days = query.days.unwrap_or(7);
    if !(1..=30).contains(&days) {
        return Err(AppError::validation_field("days", "days 必须在 1–30 之间"));
    }
    let pool = db(state)?;
    let today = today_shanghai();
    let first_day = today - ChronoDuration::days(days - 1);
    let start = shanghai_midnight_utc(first_day)?;
    let end = shanghai_midnight_utc(today + ChronoDuration::days(1))?;
    let base_filters = AggregateFilters {
        start,
        end,
        user_id,
        key_id: None,
        provider_id: None,
        model_alias: None,
        exclude_days: Vec::new(),
    };

    let mut tx = begin_read_tx(pool).await?;
    // 已结账日聚合；rollup 不可用（表/数据异常）时降级为纯明细（可用性优先，日志告警）。
    let (rollup_days, rollup_rows) =
        match load_overview_rollup(&mut tx, first_day, today, &base_filters).await {
            Ok(rollup) => rollup,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "metering rollup read unavailable for overview; falling back to usage_records"
                );
                tx.rollback().await?;
                tx = begin_read_tx(pool).await?;
                (Vec::new(), Vec::new())
            }
        };
    let detail_filters = AggregateFilters {
        exclude_days: rollup_days.clone(),
        ..base_filters.clone()
    };
    let (detail_rows, _) =
        repo::aggregate_grouped(&mut tx, &detail_filters, &["day"], 1000, 0).await?;
    let mut by_day: HashMap<NaiveDate, AggregateMetrics> = HashMap::new();
    for row in detail_rows {
        if let Some(day) = row.day {
            by_day.insert(day, row.metrics);
        }
    }
    for (day, metrics) in rollup_rows {
        by_day.insert(day, metrics);
    }

    let today_start = shanghai_midnight_utc(today)?;
    let tomorrow = shanghai_midnight_utc(today + ChronoDuration::days(1))?;
    let today_filters = AggregateFilters {
        start: today_start,
        end: tomorrow,
        exclude_days: Vec::new(),
        ..base_filters.clone()
    };
    let today_aggregate = repo::aggregate_totals(&mut tx, &today_filters).await?;
    let breakdown_rows = repo::status_breakdown(&mut tx, &today_filters).await?;
    tx.commit().await?;

    // trend 按日升序（缺失日补零）；totals = 各日合并（Decimal 精确）。
    let mut trend = Vec::with_capacity(days as usize);
    let mut totals = AggregateMetrics::default();
    for offset in 0..days {
        let day = first_day + ChronoDuration::days(offset);
        let metrics = by_day.remove(&day).unwrap_or_default();
        add_metrics(&mut totals, &metrics);
        trend.push(TrendDay {
            day,
            metrics: metrics_from_aggregate(metrics),
        });
    }

    let mut today_metrics = metrics_from_aggregate(today_aggregate);
    today_metrics.error_rate = Some(error_rate(
        today_metrics.error_requests,
        today_metrics.requests,
    ));
    let mut breakdown = serde_json::Map::new();
    for status in [
        "success",
        "upstream_error",
        "timeout",
        "gateway_error",
        "client_disconnect",
        "rejected",
    ] {
        breakdown.insert(status.to_string(), serde_json::json!(0));
    }
    for (status, count) in breakdown_rows {
        breakdown.insert(status, serde_json::json!(count));
    }

    Ok(OverviewResponse {
        currency: CURRENCY_CNY,
        timezone: OVERVIEW_TIMEZONE,
        generated_at: Utc::now(),
        days,
        today: TodayMetrics {
            day: today,
            metrics: today_metrics,
            status_breakdown: breakdown,
        },
        trend,
        totals: metrics_from_aggregate(totals),
    })
}

/// `/api/admin/usage`：按 day/model_alias/key_id 聚合 + 全区间 totals。
///
/// S5.1：区间按上海日拆分——整日已 finalized 的日读 rollup，其余（含今天）读明细；
/// 合并分组键后内存排序/分页，`total` 为合并后的分组数，形状/金额/分页语义不变。
pub async fn usage(state: &AppState, query: &UsageQuery) -> Result<UsageResponse, AppError> {
    let group_by = parse_group_by(query.group_by.as_deref())?;
    let today = today_shanghai();
    let default_from = shanghai_midnight_utc(today - ChronoDuration::days(6))?;
    let from = query.from.unwrap_or(default_from);
    let to = query.to.unwrap_or_else(Utc::now);
    if from >= to {
        return Err(AppError::bad_request(
            "invalid_time_range",
            "from 必须早于 to",
        ));
    }
    if to - from > ChronoDuration::days(366) {
        return Err(AppError::bad_request(
            "invalid_time_range",
            "查询跨度不能超过 366 天",
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
    let model_alias = match query.model_alias.as_deref().map(str::trim) {
        Some("") => None,
        Some(value) if value.chars().count() > 100 => {
            return Err(AppError::validation_field(
                "model_alias",
                "model_alias 不能超过 100 个字符",
            ));
        }
        Some(value) => Some(value.to_string()),
        None => None,
    };
    let pool = db(state)?;
    let filters = AggregateFilters {
        start: from,
        end: to,
        user_id: query.user_id,
        key_id: query.key_id,
        provider_id: query.provider_id,
        model_alias,
        exclude_days: Vec::new(),
    };

    let mut tx = begin_read_tx(pool).await?;
    let rollup = match load_usage_rollup(&mut tx, from, to, today, &filters, &group_by).await {
        Ok(rollup) => rollup,
        Err(error) => {
            tracing::warn!(
                %error,
                "metering rollup read unavailable for usage; falling back to usage_records"
            );
            tx.rollback().await?;
            tx = begin_read_tx(pool).await?;
            UsageRollupRead::default()
        }
    };

    // 无已结账日：沿用 S5 明细直查（DB 分页/排序，行为不变）。
    if rollup.days.is_empty() {
        let totals = repo::aggregate_totals(&mut tx, &filters).await?;
        let (rows, total) =
            repo::aggregate_grouped(&mut tx, &filters, &group_by, limit, offset).await?;
        tx.commit().await?;
        let items = usage_items_from_groups(rows, &group_by);
        return Ok(UsageResponse {
            currency: CURRENCY_CNY,
            from,
            to,
            group_by,
            totals: metrics_from_aggregate(totals),
            items,
            total,
            limit,
            offset,
        });
    }

    let detail_filters = AggregateFilters {
        exclude_days: rollup.days.clone(),
        ..filters.clone()
    };
    let detail_rows = repo::aggregate_grouped_all(&mut tx, &detail_filters, &group_by).await?;
    tx.commit().await?;

    let merged = merge_group_aggregates(detail_rows, rollup.rows);
    let mut totals = AggregateMetrics::default();
    for (_, metrics) in &merged {
        add_metrics(&mut totals, metrics);
    }
    let total = i64::try_from(merged.len()).unwrap_or(i64::MAX);
    let items = merged
        .into_iter()
        .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        .take(usize::try_from(limit).unwrap_or(usize::MAX))
        .map(|(key, metrics)| usage_item_from_group(key.0, key.1, key.2, metrics, &group_by))
        .collect();
    Ok(UsageResponse {
        currency: CURRENCY_CNY,
        from,
        to,
        group_by,
        totals: metrics_from_aggregate(totals),
        items,
        total,
        limit,
        offset,
    })
}

/// overview 的已结账日聚合（整窗对齐日边界，finalized 日即完整日）。
async fn load_overview_rollup(
    conn: &mut PgConnection,
    first_day: NaiveDate,
    today: NaiveDate,
    filters: &AggregateFilters,
) -> Result<(Vec<NaiveDate>, Vec<(NaiveDate, AggregateMetrics)>), AppError> {
    let days = repo::finalized_days(conn, first_day, today).await?;
    let rows = repo::rollup_aggregate_days(conn, &days, filters).await?;
    Ok((days, rows))
}

/// usage 的已结账日读取结果：覆盖日集合 + 对应 rollup 分组行。
#[derive(Debug, Default)]
struct UsageRollupRead {
    days: Vec<NaiveDate>,
    rows: Vec<GroupAggregate>,
}

async fn load_usage_rollup(
    conn: &mut PgConnection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    today: NaiveDate,
    filters: &AggregateFilters,
    group_by: &[&str],
) -> Result<UsageRollupRead, AppError> {
    let days = covered_finalized_days(conn, from, to, today).await?;
    let rows = repo::rollup_aggregate_grouped(conn, &days, filters, group_by).await?;
    Ok(UsageRollupRead { days, rows })
}

/// 区间内“整日被 finalized 覆盖”的上海日（其余日继续读明细）。
async fn covered_finalized_days(
    conn: &mut PgConnection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    today: NaiveDate,
) -> Result<Vec<NaiveDate>, AppError> {
    let offset = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS).expect("valid +08:00");
    let first_day = from.with_timezone(&offset).date_naive();
    let last_day = (to - chrono::Duration::nanoseconds(1))
        .with_timezone(&offset)
        .date_naive();
    if first_day > last_day || first_day >= today {
        return Ok(Vec::new());
    }
    let query_end = today.min(last_day.succ_opt().unwrap_or(last_day));
    let finalized: HashSet<NaiveDate> = repo::finalized_days(conn, first_day, query_end)
        .await?
        .into_iter()
        .collect();
    let mut days = Vec::new();
    let mut day = first_day;
    while day <= last_day {
        if day < today && finalized.contains(&day) && day_fully_covered(day, from, to)? {
            days.push(day);
        }
        let Some(next) = day.succ_opt() else { break };
        day = next;
    }
    Ok(days)
}

/// 上海日 `day` 是否完整落在半开区间 `[from, to)` 内（部分边界日仍读明细，避免多算）。
fn day_fully_covered(
    day: NaiveDate,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<bool, AppError> {
    let day_start = shanghai_midnight_utc(day)?;
    let Some(next) = day.succ_opt() else {
        return Ok(false);
    };
    let day_end = shanghai_midnight_utc(next)?;
    Ok(from <= day_start && day_end <= to)
}

type GroupKey = (Option<NaiveDate>, Option<String>, Option<Uuid>);

/// 合并 finalized rollup 行与明细行：按分组键 SUM，最后按 (day, model_alias, key_id) 升序。
fn merge_group_aggregates(
    detail: Vec<GroupAggregate>,
    rollup: Vec<GroupAggregate>,
) -> Vec<(GroupKey, AggregateMetrics)> {
    let mut merged: HashMap<GroupKey, AggregateMetrics> = HashMap::new();
    for row in detail.into_iter().chain(rollup) {
        let key = (row.day, row.model_alias, row.key_id);
        match merged.entry(key) {
            Entry::Occupied(mut entry) => add_metrics(entry.get_mut(), &row.metrics),
            Entry::Vacant(entry) => {
                entry.insert(row.metrics);
            }
        }
    }
    let mut groups: Vec<(GroupKey, AggregateMetrics)> = merged.into_iter().collect();
    groups.sort_by(|left, right| left.0.cmp(&right.0));
    groups
}

fn add_metrics(target: &mut AggregateMetrics, source: &AggregateMetrics) {
    target.requests += source.requests;
    target.success_requests += source.success_requests;
    target.error_requests += source.error_requests;
    target.rejected_requests += source.rejected_requests;
    target.client_disconnect_requests += source.client_disconnect_requests;
    target.input_tokens += source.input_tokens;
    target.output_tokens += source.output_tokens;
    target.cost += source.cost;
}

fn usage_items_from_groups(rows: Vec<GroupAggregate>, group_by: &[&str]) -> Vec<UsageItem> {
    rows.into_iter()
        .map(|row| {
            usage_item_from_group(row.day, row.model_alias, row.key_id, row.metrics, group_by)
        })
        .collect()
}

fn usage_item_from_group(
    day: Option<NaiveDate>,
    model_alias: Option<String>,
    key_id: Option<Uuid>,
    metrics: AggregateMetrics,
    group_by: &[&str],
) -> UsageItem {
    UsageItem {
        day: if group_by.contains(&"day") { day } else { None },
        model_alias: if group_by.contains(&"model_alias") {
            model_alias
        } else {
            None
        },
        key_id: if group_by.contains(&"key_id") {
            key_id
        } else {
            None
        },
        metrics: metrics_from_aggregate(metrics),
    }
}

fn parse_group_by(raw: Option<&str>) -> Result<Vec<&'static str>, AppError> {
    let raw = raw.unwrap_or("day").trim();
    if raw.is_empty() {
        return Err(AppError::bad_request(
            "invalid_group_by",
            "group_by 不能为空",
        ));
    }
    let mut requested = std::collections::HashSet::new();
    for part in raw.split(',') {
        let part = part.trim();
        if !matches!(part, "day" | "model_alias" | "key_id") {
            return Err(AppError::bad_request(
                "invalid_group_by",
                format!("不支持的 group_by: {part}"),
            ));
        }
        requested.insert(part);
    }
    let mut out = Vec::new();
    for dim in ["day", "model_alias", "key_id"] {
        if requested.contains(dim) {
            out.push(dim);
        }
    }
    if out.is_empty() {
        return Err(AppError::bad_request(
            "invalid_group_by",
            "group_by 不能为空",
        ));
    }
    Ok(out)
}

fn metrics_from_aggregate(row: AggregateMetrics) -> Metrics {
    Metrics {
        requests: row.requests,
        success_requests: row.success_requests,
        error_requests: row.error_requests,
        rejected_requests: row.rejected_requests,
        client_disconnect_requests: row.client_disconnect_requests,
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cost: format_decimal(row.cost),
        error_rate: None,
    }
}

fn format_decimal(value: Decimal) -> String {
    format!("{value:.6}")
}

fn format_cost(raw: &str) -> String {
    Decimal::from_str(raw)
        .map(format_decimal)
        .unwrap_or_else(|_| "0.000000".to_string())
}

fn error_rate(errors: i64, requests: i64) -> f64 {
    if requests <= 0 {
        0.0
    } else {
        (errors as f64 / requests as f64 * 1_000_000.0).round() / 1_000_000.0
    }
}

fn today_shanghai() -> NaiveDate {
    let offset = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS).expect("valid +08:00");
    Utc::now().with_timezone(&offset).date_naive()
}

fn shanghai_midnight_utc(day: NaiveDate) -> Result<DateTime<Utc>, AppError> {
    let offset = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS).expect("valid +08:00");
    let local = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::internal("invalid Shanghai day"))?;
    offset
        .from_local_datetime(&local)
        .single()
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| AppError::internal("invalid Shanghai midnight"))
}

/// 上海日 → UTC 半开区间 `[start, end)`。
fn shanghai_day_utc_bounds(day: NaiveDate) -> Result<(DateTime<Utc>, DateTime<Utc>), AppError> {
    let start = shanghai_midnight_utc(day)?;
    let next = day
        .succ_opt()
        .ok_or_else(|| AppError::internal("invalid Shanghai day successor"))?;
    Ok((start, shanghai_midnight_utc(next)?))
}

// ---------- S5.1 日结 ----------

/// 日结单日（Asia/Shanghai 已完成日）：整日全量重算 + 事务替换 + 平衡校验。
///
/// - `day >= today`（今天/未来）→ `Ok(false)`（当日不结账，直读明细）；
/// - 幂等：同输入重复日结结果相同（DELETE+INSERT+UPSERT）；
/// - 失败：事务回滚，该日保持未 finalized，读路径回退明细；下轮 tick 重试。
pub async fn finalize_day(state: &AppState, day: NaiveDate) -> Result<bool, AppError> {
    let pool = db(state)?;
    let today = today_shanghai();
    if day >= today {
        return Ok(false);
    }
    let (start, end) = shanghai_day_utc_bounds(day)?;
    let mut tx = pool.begin().await?;
    repo::lock_rollup_day(&mut tx, day).await?;
    repo::delete_rollup_day(&mut tx, day).await?;
    repo::insert_rollup_day(&mut tx, day, start, end).await?;
    let (source_records, source_cost) = repo::source_totals(&mut tx, start, end).await?;
    let (rollup_records, rollup_cost) = repo::rollup_totals(&mut tx, day).await?;
    if source_records != rollup_records || source_cost != rollup_cost {
        // 未提交即返回 → 事务被 drop 回滚；该日保持未 finalized（DB balanced CHECK 兜底）。
        return Err(AppError::internal(format!(
            "metering rollup unbalanced for {day}: source={source_records}/{source_cost}, \
             rollup={rollup_records}/{rollup_cost}"
        )));
    }
    repo::upsert_rollup_day(
        &mut tx,
        day,
        source_records,
        source_cost,
        rollup_records,
        rollup_cost,
    )
    .await?;
    tx.commit().await?;
    tracing::info!(%day, requests = source_records, cost = %source_cost, "metering rollup finalized");
    Ok(true)
}

/// 结算到期日（`< today` 且未 finalized），每 tick 最多 `max_days` 天。
///
/// 单日失败只记日志并继续（保持未 finalized，下轮重试）；返回成功结账天数。
pub async fn finalize_due_days(state: &AppState, max_days: usize) -> Result<usize, AppError> {
    let pool = db(state)?;
    if max_days == 0 {
        return Ok(0);
    }
    let today = today_shanghai();
    let today_start = shanghai_midnight_utc(today)?;
    let limit = i64::try_from(max_days).unwrap_or(i64::MAX);
    let days = repo::due_finalize_days(pool, today, today_start, limit).await?;
    let mut finalized = 0;
    for day in days {
        match finalize_day(state, day).await {
            Ok(true) => finalized += 1,
            Ok(false) => {}
            Err(error) => {
                tracing::error!(%error, %day, "metering finalize day failed; retry next tick");
            }
        }
    }
    Ok(finalized)
}

// ---------- S5.1 维护任务：日结 → 明细清理 → rollup 清理 ----------

/// 维护一次（测试/手动/后台 tick 共用）：错误只记日志，下轮重试。
pub async fn run_maintenance_once(state: &AppState) {
    if state.db.is_none() {
        return;
    }
    match finalize_due_days(state, FINALIZE_MAX_DAYS_PER_TICK).await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "metering rollup finalized due days"),
        Err(error) => tracing::error!(%error, "metering rollup finalize failed; retry next tick"),
    }
    match cleanup_expired_details(state).await {
        Ok(0) => {}
        Ok(deleted) => {
            tracing::info!(
                deleted,
                "metering usage detail retention cleanup deleted rows"
            )
        }
        Err(error) => {
            tracing::error!(%error, "metering usage detail cleanup failed; retry next tick")
        }
    }
    match cleanup_expired_rollups(state).await {
        Ok((0, 0)) => {}
        Ok((rollups, days)) => {
            tracing::info!(
                rollups,
                days,
                "metering rollup retention cleanup deleted rows"
            )
        }
        Err(error) => tracing::error!(%error, "metering rollup cleanup failed; retry next tick"),
    }
}

/// 后台维护任务：启动后立即执行一次（日结 backfill），随后按 `rollup_interval_secs` tick。
///
/// 维护任务不进入请求关键路径；优雅退出无需 flush（日结/清理均幂等、单语句可重试）。
pub fn spawn_maintenance(state: AppState) {
    if state.db.is_none() {
        tracing::warn!("metering maintenance not started: database disabled");
        return;
    }
    let interval_secs = state.config.rollup_interval_secs;
    let tick_state = state.clone();
    tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs);
        loop {
            run_maintenance_once(&tick_state).await;
            tokio::time::sleep(interval).await;
        }
    });
    tracing::info!(interval_secs, "metering maintenance task started");
}

/// 明细保留清理：仅删已 finalized 且 `created_at < now - usage_retention_days` 的行。
async fn cleanup_expired_details(state: &AppState) -> Result<u64, AppError> {
    let pool = db(state)?;
    let cutoff = Utc::now() - ChronoDuration::days(state.config.usage_retention_days);
    let mut total = 0u64;
    for _ in 0..DETAIL_CLEANUP_MAX_BATCHES {
        let deleted = repo::delete_expired_detail_batch(pool, cutoff, DETAIL_CLEANUP_BATCH).await?;
        total += deleted;
        if deleted < DETAIL_CLEANUP_BATCH as u64 {
            break;
        }
    }
    Ok(total)
}

/// rollup/日结元数据保留清理：`day < today - rollup_retention_days`。
async fn cleanup_expired_rollups(state: &AppState) -> Result<(u64, u64), AppError> {
    let pool = db(state)?;
    let cutoff_day = today_shanghai() - ChronoDuration::days(state.config.rollup_retention_days);
    repo::delete_expired_rollups(pool, cutoff_day).await
}

/// 单条用量读取（observability 请求详情合并；不存在/无用量返回 Ok(None)）。
pub async fn get_usage_by_request_id(
    state: &AppState,
    request_id: &str,
) -> Result<Option<super::model::UsageBlock>, AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(AppError::database_unavailable)?;
    let Some(row) = repo::get_usage_by_request_id(pool, request_id).await? else {
        return Ok(None);
    };
    Ok(Some(super::model::UsageBlock {
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cost: format_cost(&row.cost),
        currency: row.currency,
        latency_ms: row.latency_ms,
        ttft_ms: row.ttft_ms,
        is_stream: row.is_stream,
        fallback: row.fallback,
        status: row.status,
        http_status: row.http_status,
        error_code: row.error_code,
        usage_source: row.usage_source,
    }))
}

/// 单测/集成测试用空事件（字段取值均在约束内）。
#[cfg(test)]
pub(crate) fn test_event(
    user_id: Uuid,
    device_id: Uuid,
    key_id: Uuid,
    request_id: &str,
    created_at: DateTime<Utc>,
) -> UsageEvent {
    UsageEvent {
        request_id: request_id.to_string(),
        created_at,
        user_id,
        device_id,
        key_id,
        model_alias: "fast".to_string(),
        resolved_upstream_model: Some("deepseek-chat".to_string()),
        provider_id: None,
        provider_name: Some("TestProvider".to_string()),
        deployment_id: None,
        source: super::model::RouteSource::Alias,
        is_stream: true,
        fallback: false,
        attempt_count: 1,
        input_tokens: Some(1_000_000),
        output_tokens: Some(500_000),
        usage_source: super::model::UsageSource::Upstream,
        price_input: Some(Decimal::from(3)),
        price_output: Some(Decimal::from(15)),
        price_currency: Some(PRICE_CURRENCY_CNY.to_string()),
        http_status: 200,
        status: super::model::UsageStatus::Success,
        error_code: None,
        latency_ms: 1_200,
        ttft_ms: Some(300),
        currency: CURRENCY_CNY.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::UsageStatus;
    use super::*;
    use crate::modules::metering::repo::{AggregateMetrics, GroupAggregate, ZERO_PROVIDER_ID};
    use crate::platform::{config::Config, crypto::Crypto};
    use chrono::NaiveDate;
    use sqlx::Row;
    use std::collections::HashSet;
    use std::{str::FromStr, sync::Arc, time::Instant};

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
            crypto: Arc::new(Crypto::from_key_bytes([9u8; 32])),
            http: crate::platform::http::shared_client(),
            started_at: Instant::now(),
        }
    }

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn shanghai_day_bounds_uses_fixed_plus_eight() {
        // 2026-10-30 15:59:59Z = 上海 23:59:59 → 当日起点 10-29T16:00:00Z。
        let (start, end) = shanghai_day_bounds(utc(2026, 10, 30, 15, 59, 59));
        assert_eq!(start, utc(2026, 10, 29, 16, 0, 0));
        assert_eq!(end, utc(2026, 10, 30, 16, 0, 0));

        // 2026-10-30 16:00:00Z = 上海 10-31 00:00 → 新的一天起点。
        let (start, end) = shanghai_day_bounds(utc(2026, 10, 30, 16, 0, 0));
        assert_eq!(start, utc(2026, 10, 30, 16, 0, 0));
        assert_eq!(end, utc(2026, 10, 31, 16, 0, 0));

        // 跨月/跨年：2026-12-31T16:30:00Z = 上海 2027-01-01 00:30。
        let (start, end) = shanghai_day_bounds(utc(2026, 12, 31, 16, 30, 0));
        assert_eq!(start, utc(2026, 12, 31, 16, 0, 0));
        assert_eq!(end, utc(2027, 1, 1, 16, 0, 0));
    }

    #[tokio::test]
    async fn flush_returns_true_once_in_flight_drains() {
        // 全局 in-flight 计数可能被同模块并行测试占用，给足排空时间。
        assert!(flush(Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn record_usage_without_db_fails_closed() {
        let state = test_state(None);
        let event = test_event(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "req-no-db",
            Utc::now(),
        );
        assert!(matches!(
            record_usage(&state, event).await,
            Err(AppError::DatabaseUnavailable)
        ));
    }

    #[tokio::test]
    async fn detached_write_drains_through_flush() {
        let state = test_state(None);
        let event = test_event(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "req-detached",
            Utc::now(),
        );
        record_usage_detached(&state, event);
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
        // 所有用例使用随机 user/request_id，断言只针对本用例数据。
        Some(test_state(Some(pool)))
    }

    async fn seed_identity(state: &AppState) -> (Uuid, Uuid, Uuid) {
        let pool = state.db.as_ref().expect("test pool");
        let user_id = Uuid::new_v4();
        let device_id = Uuid::new_v4();
        let key_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, email) VALUES ($1, $2)")
            .bind(user_id)
            .bind(format!("{user_id}@corp.example.com"))
            .execute(pool)
            .await
            .expect("insert user");
        sqlx::query(
            "INSERT INTO devices (id, user_id, name, platform, device_secret_hash) \
             VALUES ($1, $2, 'test', 'test', $3)",
        )
        .bind(device_id)
        .bind(user_id)
        .bind(format!("secret-{device_id}"))
        .execute(pool)
        .await
        .expect("insert device");
        sqlx::query(
            "INSERT INTO virtual_keys (id, device_id, user_id, key_prefix, key_hash) \
             VALUES ($1, $2, $3, 'sk-gw-test', $4)",
        )
        .bind(key_id)
        .bind(device_id)
        .bind(user_id)
        .bind(format!("hash-{key_id}"))
        .execute(pool)
        .await
        .expect("insert virtual key");
        (user_id, device_id, key_id)
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 usage_records；用例使用随机 id，不 TRUNCATE）"]
    async fn record_usage_is_idempotent_and_maps_fields() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let request_id = format!("req-{}", Uuid::new_v4());
        let event = test_event(user_id, device_id, key_id, &request_id, Utc::now());

        record_usage(&state, event.clone())
            .await
            .expect("first insert");
        // 重复写入被唯一键吸收，仍返回 Ok 且只落一行。
        record_usage(&state, event).await.expect("duplicate insert");

        let pool = state.db.as_ref().expect("pool");
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM usage_records WHERE request_id = $1")
                .bind(&request_id)
                .fetch_one(pool)
                .await
                .expect("count rows");
        assert_eq!(count, 1);

        let row: (
            String,
            i32,
            i32,
            String,
            String,
            String,
            String,
            Option<i32>,
            bool,
        ) = sqlx::query_as(
            "SELECT cost::text, input_tokens, output_tokens, status, usage_source, \
                        price_currency, source, ttft_ms, is_stream \
                 FROM usage_records WHERE request_id = $1",
        )
        .bind(&request_id)
        .fetch_one(pool)
        .await
        .expect("fetch row");
        // 1M 输入 @3 + 500k 输出 @15 = 3 + 7.5 = 10.5 CNY。
        assert_eq!(
            Decimal::from_str(&row.0).expect("cost decimal"),
            Decimal::from_str("10.5").expect("expected cost")
        );
        assert_eq!(row.1, 1_000_000);
        assert_eq!(row.2, 500_000);
        assert_eq!(row.3, "success");
        assert_eq!(row.4, "upstream");
        assert_eq!(row.5, "CNY");
        assert_eq!(row.6, "alias");
        assert_eq!(row.7, Some(300));
        assert!(row.8);
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 usage_records；用例使用随机 id，不 TRUNCATE）"]
    async fn today_spend_only_counts_shanghai_today_for_user() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let (other_user, other_device, other_key) = seed_identity(&state).await;

        let now = Utc::now();
        let suffix = Uuid::new_v4();
        // 今日 10.5 CNY。
        let today = test_event(
            user_id,
            device_id,
            key_id,
            &format!("req-today-{suffix}"),
            now,
        );
        record_usage(&state, today).await.expect("today insert");
        // 昨日（同用户）不计入。
        let yesterday = test_event(
            user_id,
            device_id,
            key_id,
            &format!("req-yesterday-{suffix}"),
            now - chrono::Duration::days(1),
        );
        record_usage(&state, yesterday)
            .await
            .expect("yesterday insert");
        // 其他用户今日不计入。
        let other = test_event(other_user, other_device, other_key, "req-other", now);
        record_usage(&state, other).await.expect("other insert");
        // rejected 行（cost=0）不影响金额。
        let mut rejected = test_event(user_id, device_id, key_id, "req-rejected", now);
        rejected.status = UsageStatus::Rejected;
        rejected.input_tokens = Some(0);
        rejected.output_tokens = Some(0);
        rejected.price_input = None;
        rejected.price_output = None;
        rejected.price_currency = None;
        record_usage(&state, rejected)
            .await
            .expect("rejected insert");

        let spent = today_spend(&state, user_id).await.expect("today spend");
        assert_eq!(spent, Decimal::from_str("10.5").expect("expected spend"));
        // 无数据用户 = 0（不返回错误）。
        let empty = today_spend(&state, Uuid::new_v4())
            .await
            .expect("empty spend");
        assert_eq!(empty, Decimal::ZERO);
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 usage_records；用例使用随机 id，不 TRUNCATE）"]
    async fn non_stream_event_drops_ttft_and_invalid_http_status() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let request_id = format!("req-non-stream-{}", Uuid::new_v4());
        let mut event = test_event(user_id, device_id, key_id, &request_id, Utc::now());
        event.is_stream = false;
        event.ttft_ms = Some(120);
        event.http_status = 0;
        record_usage(&state, event).await.expect("insert");

        let row: (Option<i32>, Option<i32>) =
            sqlx::query_as("SELECT ttft_ms, http_status FROM usage_records WHERE request_id = $1")
                .bind(&request_id)
                .fetch_one(state.db.as_ref().expect("pool"))
                .await
                .expect("fetch row");
        assert_eq!(row, (None, None));
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（写 usage_records；用例使用随机 id，不 TRUNCATE）"]
    async fn usd_price_snapshots_fx_and_converts_to_cny() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let request_id = format!("req-usd-{}", Uuid::new_v4());
        let mut event = test_event(user_id, device_id, key_id, &request_id, Utc::now());
        event.input_tokens = Some(1_000_000);
        event.output_tokens = Some(0);
        event.price_input = Some(Decimal::from(1));
        event.price_output = None;
        event.price_currency = Some(PRICE_CURRENCY_USD.to_string());
        record_usage(&state, event).await.expect("insert");

        let row: (String, String, Option<String>) = sqlx::query_as(
            "SELECT cost::text, price_currency, fx_rate::text \
             FROM usage_records WHERE request_id = $1",
        )
        .bind(&request_id)
        .fetch_one(state.db.as_ref().expect("pool"))
        .await
        .expect("fetch row");
        // 1 USD @7.2 = 7.2 CNY。
        assert_eq!(
            Decimal::from_str(&row.0).expect("cost"),
            Decimal::from_str("7.2").expect("expected")
        );
        assert_eq!(row.1, "USD");
        assert_eq!(
            row.2.map(|value| Decimal::from_str(&value).expect("fx")),
            Some(Decimal::from_str("7.2").expect("expected fx"))
        );
    }

    // ---------- S5.1 单元测试（纯函数/边界） ----------

    #[test]
    fn day_fully_covered_requires_whole_shanghai_day() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let start = shanghai_midnight_utc(day).unwrap();
        let next = day.succ_opt().unwrap();
        let end = shanghai_midnight_utc(next).unwrap();
        assert!(day_fully_covered(day, start, end).unwrap());
        assert!(!day_fully_covered(day, start + chrono::Duration::minutes(1), end).unwrap());
        assert!(!day_fully_covered(day, start, end - chrono::Duration::minutes(1)).unwrap());
    }

    #[test]
    fn merge_group_aggregates_sums_and_sorts() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let key = Uuid::new_v4();
        let detail = vec![GroupAggregate {
            day: Some(day),
            model_alias: Some("smart".to_string()),
            key_id: Some(key),
            metrics: AggregateMetrics {
                requests: 1,
                cost: Decimal::ONE,
                ..Default::default()
            },
        }];
        let rollup = vec![
            GroupAggregate {
                day: Some(day),
                model_alias: Some("fast".to_string()),
                key_id: Some(key),
                metrics: AggregateMetrics {
                    requests: 2,
                    success_requests: 1,
                    cost: Decimal::new(15, 1),
                    ..Default::default()
                },
            },
            GroupAggregate {
                day: Some(day),
                model_alias: Some("smart".to_string()),
                key_id: Some(key),
                metrics: AggregateMetrics {
                    requests: 3,
                    input_tokens: 10,
                    cost: Decimal::new(25, 1),
                    ..Default::default()
                },
            },
        ];
        let merged = merge_group_aggregates(detail, rollup);
        assert_eq!(merged.len(), 2);
        // 固定维度序 day/model_alias/key_id 升序：fast < smart。
        assert_eq!(merged[0].0.1.as_deref(), Some("fast"));
        assert_eq!(merged[0].1.requests, 2);
        assert_eq!(merged[1].0.1.as_deref(), Some("smart"));
        assert_eq!(merged[1].1.requests, 4);
        assert_eq!(merged[1].1.input_tokens, 10);
        assert_eq!(merged[1].1.cost, Decimal::new(35, 1));
    }

    // ---------- S5.1 gated helpers ----------

    fn metrics_from_row(row: &sqlx::postgres::PgRow) -> AggregateMetrics {
        AggregateMetrics {
            requests: row.get("requests"),
            success_requests: row.get("success_requests"),
            error_requests: row.get("error_requests"),
            rejected_requests: row.get("rejected_requests"),
            client_disconnect_requests: row.get("client_disconnect_requests"),
            input_tokens: row.get("input_tokens"),
            output_tokens: row.get("output_tokens"),
            cost: Decimal::from_str(&row.get::<String, _>("cost")).expect("cost decimal"),
        }
    }

    async fn seed_provider(state: &AppState) -> Uuid {
        let pool = state.db.as_ref().expect("pool");
        let provider_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO providers (id, name, kind, base_url) \
             VALUES ($1, $2, 'openai_compatible', 'http://127.0.0.1:1/v1')",
        )
        .bind(provider_id)
        .bind(format!("metering-s51-{provider_id}"))
        .execute(pool)
        .await
        .expect("insert provider");
        provider_id
    }

    #[allow(clippy::too_many_arguments)] // 测试种子构造，字段语义清晰
    async fn seed_usage(
        state: &AppState,
        user_id: Uuid,
        device_id: Uuid,
        key_id: Uuid,
        provider_id: Option<Uuid>,
        created_at: DateTime<Utc>,
        request_id: &str,
        status: UsageStatus,
    ) {
        let mut event = test_event(user_id, device_id, key_id, request_id, created_at);
        event.provider_id = provider_id;
        event.provider_name = provider_id.map(|_| "MeteringS51Provider".to_string());
        event.status = status;
        if status == UsageStatus::Success {
            // 1M 输入 @1 CNY/1M = 1.000000 CNY（便于断言）。
            event.input_tokens = Some(1_000_000);
            event.output_tokens = Some(0);
            event.price_input = Some(Decimal::ONE);
            event.price_output = None;
            event.price_currency = Some(PRICE_CURRENCY_CNY.to_string());
        } else {
            event.input_tokens = Some(0);
            event.output_tokens = Some(0);
            event.price_input = None;
            event.price_output = None;
            event.price_currency = None;
        }
        record_usage(state, event).await.expect("record usage");
    }

    async fn cleanup_rollup_days(state: &AppState, days: &[NaiveDate]) {
        let Some(pool) = state.db.as_ref() else {
            return;
        };
        if days.is_empty() {
            return;
        }
        sqlx::query("DELETE FROM daily_usage_rollups WHERE day = ANY($1)")
            .bind(days)
            .execute(pool)
            .await
            .expect("cleanup rollups");
        sqlx::query("DELETE FROM usage_rollup_days WHERE day = ANY($1)")
            .bind(days)
            .execute(pool)
            .await
            .expect("cleanup rollup days");
    }

    async fn cleanup_users_and_usage(state: &AppState, user_ids: &[Uuid], provider_ids: &[Uuid]) {
        let Some(pool) = state.db.as_ref() else {
            return;
        };
        if !user_ids.is_empty() {
            for sql in [
                "DELETE FROM usage_records WHERE user_id = ANY($1)",
                "DELETE FROM virtual_keys WHERE user_id = ANY($1)",
                "DELETE FROM devices WHERE user_id = ANY($1)",
                "DELETE FROM budgets WHERE user_id = ANY($1)",
                "DELETE FROM users WHERE id = ANY($1)",
            ] {
                sqlx::query(sql)
                    .bind(user_ids)
                    .execute(pool)
                    .await
                    .expect("cleanup test users");
            }
        }
        if !provider_ids.is_empty() {
            sqlx::query("DELETE FROM providers WHERE id = ANY($1)")
                .bind(provider_ids)
                .execute(pool)
                .await
                .expect("cleanup test providers");
        }
    }

    async fn finalized_day_set(state: &AppState) -> HashSet<NaiveDate> {
        let pool = state.db.as_ref().expect("pool");
        let days: Vec<NaiveDate> = sqlx::query_scalar(
            "SELECT day FROM usage_rollup_days WHERE finalized = true ORDER BY day",
        )
        .fetch_all(pool)
        .await
        .expect("finalized days");
        days.into_iter().collect()
    }

    async fn sql_detail_metrics(
        pool: &sqlx::PgPool,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AggregateMetrics {
        let row = sqlx::query(
            "SELECT count(*) AS requests, \
                 count(*) FILTER (WHERE status = 'success') AS success_requests, \
                 count(*) FILTER (WHERE status IN ('upstream_error', 'timeout', 'gateway_error')) AS error_requests, \
                 count(*) FILTER (WHERE status = 'rejected') AS rejected_requests, \
                 count(*) FILTER (WHERE status = 'client_disconnect') AS client_disconnect_requests, \
                 coalesce(sum(input_tokens), 0)::bigint AS input_tokens, \
                 coalesce(sum(output_tokens), 0)::bigint AS output_tokens, \
                 coalesce(sum(cost), 0)::text AS cost \
             FROM usage_records WHERE created_at >= $1 AND created_at < $2",
        )
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await
        .expect("detail metrics");
        metrics_from_row(&row)
    }

    async fn sql_rollup_metrics(pool: &sqlx::PgPool, day: NaiveDate) -> AggregateMetrics {
        let row = sqlx::query(
            "SELECT coalesce(sum(requests), 0)::bigint AS requests, \
                 coalesce(sum(success_requests), 0)::bigint AS success_requests, \
                 coalesce(sum(error_requests), 0)::bigint AS error_requests, \
                 coalesce(sum(rejected_requests), 0)::bigint AS rejected_requests, \
                 coalesce(sum(client_disconnect_requests), 0)::bigint AS client_disconnect_requests, \
                 coalesce(sum(input_tokens), 0)::bigint AS input_tokens, \
                 coalesce(sum(output_tokens), 0)::bigint AS output_tokens, \
                 coalesce(sum(cost), 0)::text AS cost \
             FROM daily_usage_rollups WHERE day = $1",
        )
        .bind(day)
        .fetch_one(pool)
        .await
        .expect("rollup metrics");
        metrics_from_row(&row)
    }

    fn assert_metrics_eq(actual: &Metrics, expected: &AggregateMetrics) {
        assert_eq!(actual.requests, expected.requests, "requests");
        assert_eq!(
            actual.success_requests, expected.success_requests,
            "success_requests"
        );
        assert_eq!(
            actual.error_requests, expected.error_requests,
            "error_requests"
        );
        assert_eq!(
            actual.rejected_requests, expected.rejected_requests,
            "rejected_requests"
        );
        assert_eq!(
            actual.client_disconnect_requests, expected.client_disconnect_requests,
            "client_disconnect_requests"
        );
        assert_eq!(actual.input_tokens, expected.input_tokens, "input_tokens");
        assert_eq!(
            actual.output_tokens, expected.output_tokens,
            "output_tokens"
        );
        assert_eq!(actual.cost, format!("{:.6}", expected.cost), "cost");
    }

    async fn count_user_usage_in_day(pool: &sqlx::PgPool, user_id: Uuid, day: NaiveDate) -> i64 {
        let (start, end) = shanghai_day_utc_bounds(day).expect("day bounds");
        sqlx::query_scalar(
            "SELECT count(*) FROM usage_records \
             WHERE user_id = $1 AND created_at >= $2 AND created_at < $3",
        )
        .bind(user_id)
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await
        .expect("count usage")
    }

    // ---------- S5.1 gated：迁移 0010 ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（检查 0010 迁移后的 rollup schema）"]
    async fn migration_0010_adds_provider_dimension_to_rollup() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let pool = state.db.as_ref().expect("pool");
        let provider_column: Option<String> = sqlx::query_scalar(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_name = 'daily_usage_rollups' AND column_name = 'provider_id'",
        )
        .fetch_optional(pool)
        .await
        .expect("provider_id column");
        assert_eq!(provider_column.as_deref(), Some("provider_id"));

        let not_null: bool = sqlx::query_scalar(
            "SELECT is_nullable = 'NO' FROM information_schema.columns \
             WHERE table_name = 'daily_usage_rollups' AND column_name = 'provider_id'",
        )
        .fetch_one(pool)
        .await
        .expect("provider_id not null");
        assert!(not_null);

        let pk: Vec<String> = sqlx::query_scalar(
            "SELECT a.attname::text FROM pg_index i \
             JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
             WHERE i.indrelid = 'daily_usage_rollups'::regclass AND i.indisprimary \
             ORDER BY array_position(i.indkey, a.attnum)",
        )
        .fetch_all(pool)
        .await
        .expect("rollup pk columns");
        assert_eq!(
            pk,
            vec!["day", "user_id", "key_id", "model_alias", "provider_id"]
        );

        let index_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_indexes \
             WHERE tablename = 'daily_usage_rollups' \
               AND indexname = 'idx_daily_usage_rollups_provider_day'",
        )
        .fetch_one(pool)
        .await
        .expect("provider_day index");
        assert_eq!(index_count, 1);
    }

    // ---------- S5.1 gated：日结幂等/今天不结账 ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（会写 usage_records/daily_usage_rollups；用例使用随机 id）"]
    async fn finalize_day_is_idempotent_and_skips_today() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let provider_id = seed_provider(&state).await;
        let today = today_shanghai();
        let d1 = today - ChronoDuration::days(2);
        let d2 = today - ChronoDuration::days(1);
        cleanup_rollup_days(&state, &[d1, d2]).await;

        let noon1 = shanghai_midnight_utc(d1).expect("d1") + chrono::Duration::hours(12);
        let noon2 = shanghai_midnight_utc(d2).expect("d2") + chrono::Duration::hours(12);
        let prefix = Uuid::new_v4();
        seed_usage(
            &state,
            user_id,
            device_id,
            key_id,
            Some(provider_id),
            noon1,
            &format!("s51-fin-1-{prefix}"),
            UsageStatus::Success,
        )
        .await;
        seed_usage(
            &state,
            user_id,
            device_id,
            key_id,
            Some(provider_id),
            noon1,
            &format!("s51-fin-2-{prefix}"),
            UsageStatus::Rejected,
        )
        .await;
        seed_usage(
            &state,
            user_id,
            device_id,
            key_id,
            None,
            noon1,
            &format!("s51-fin-3-{prefix}"),
            UsageStatus::Success,
        )
        .await;
        seed_usage(
            &state,
            user_id,
            device_id,
            key_id,
            Some(provider_id),
            noon2,
            &format!("s51-fin-4-{prefix}"),
            UsageStatus::UpstreamError,
        )
        .await;

        assert!(finalize_day(&state, d1).await.expect("finalize d1"));

        let pool = state.db.as_ref().expect("pool");
        let (finalized, source_records, rollup_records, source_cost, rollup_cost): (
            bool,
            i64,
            i64,
            String,
            String,
        ) = sqlx::query_as(
            "SELECT finalized, source_records, rollup_records, \
                    source_cost::text, rollup_cost::text \
             FROM usage_rollup_days WHERE day = $1",
        )
        .bind(d1)
        .fetch_one(pool)
        .await
        .expect("rollup day metadata");
        assert!(finalized);
        assert_eq!(source_records, rollup_records);
        assert_eq!(source_cost, rollup_cost);
        let (start, end) = shanghai_day_utc_bounds(d1).expect("bounds");
        let source_metrics = sql_detail_metrics(pool, start, end).await;
        assert_eq!(source_records, source_metrics.requests);
        assert_eq!(
            Decimal::from_str(&source_cost).expect("source cost"),
            source_metrics.cost
        );

        // provider 维度：本用户 providerA 组（success+rejected）与 zero 哨兵组（success）。
        let rows: Vec<(String, i64, i64, i64, String)> = sqlx::query_as(
            "SELECT provider_id::text, requests, success_requests, rejected_requests, cost::text \
             FROM daily_usage_rollups WHERE day = $1 AND user_id = $2 ORDER BY provider_id::text",
        )
        .bind(d1)
        .bind(user_id)
        .fetch_all(pool)
        .await
        .expect("rollup rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, ZERO_PROVIDER_ID.to_string());
        assert_eq!((rows[0].1, rows[0].2, rows[0].3), (1, 1, 0));
        assert_eq!(
            Decimal::from_str(&rows[0].4).expect("zero cost"),
            Decimal::ONE
        );
        assert_eq!(rows[1].0, provider_id.to_string());
        assert_eq!((rows[1].1, rows[1].2, rows[1].3), (2, 1, 1));
        assert_eq!(
            Decimal::from_str(&rows[1].4).expect("provider cost"),
            Decimal::ONE
        );

        // 幂等：重复日结结果不变。
        assert!(finalize_day(&state, d1).await.expect("refinalize d1"));
        let rows_again: Vec<(String, i64, i64, i64, String)> = sqlx::query_as(
            "SELECT provider_id::text, requests, success_requests, rejected_requests, cost::text \
             FROM daily_usage_rollups WHERE day = $1 AND user_id = $2 ORDER BY provider_id::text",
        )
        .bind(d1)
        .bind(user_id)
        .fetch_all(pool)
        .await
        .expect("rollup rows again");
        assert_eq!(rows_again, rows);

        // 今天不结账。
        assert!(!finalize_day(&state, today).await.expect("today skipped"));
        let today_finalized: bool = sqlx::query_scalar(
            "SELECT coalesce(finalized, false) FROM usage_rollup_days WHERE day = $1",
        )
        .bind(today)
        .fetch_optional(pool)
        .await
        .expect("today metadata")
        .unwrap_or(false);
        assert!(!today_finalized);

        cleanup_rollup_days(&state, &[d1, d2]).await;
        cleanup_users_and_usage(&state, &[user_id], &[provider_id]).await;
    }

    // ---------- S5.1 gated：到期日结算（今天不结账） ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（会写 usage_records/daily_usage_rollups；用例使用随机 id）"]
    async fn finalize_due_days_finalizes_closed_days_only() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let today = today_shanghai();
        let d1 = today - ChronoDuration::days(2);
        let d2 = today - ChronoDuration::days(1);
        cleanup_rollup_days(&state, &[d1, d2]).await;
        let before = finalized_day_set(&state).await;

        let prefix = Uuid::new_v4();
        for (day, name) in [(d1, "due-1"), (d2, "due-2")] {
            let created_at =
                shanghai_midnight_utc(day).expect("midnight") + chrono::Duration::hours(9);
            seed_usage(
                &state,
                user_id,
                device_id,
                key_id,
                None,
                created_at,
                &format!("s51-{name}-{prefix}"),
                UsageStatus::Success,
            )
            .await;
        }

        // 多次 tick 直到没有到期日（兼容共享测试库中其它模块遗留的更早到期日）。
        let mut finalized = 0;
        for _ in 0..5 {
            let count = finalize_due_days(&state, 7).await.expect("finalize due");
            finalized += count;
            if count == 0 {
                break;
            }
        }
        assert!(finalized >= 2, "expected our 2 days to be finalized");
        assert_eq!(finalize_due_days(&state, 7).await.expect("no due left"), 0);
        let current = finalized_day_set(&state).await;
        assert!(current.contains(&d1));
        assert!(current.contains(&d2));
        assert!(!current.contains(&today));

        let new_days: Vec<NaiveDate> = current.difference(&before).copied().collect();
        cleanup_rollup_days(&state, &new_days).await;
        cleanup_users_and_usage(&state, &[user_id], &[]).await;
    }

    // ---------- S5.1 gated：overview 合并（rollup + 明细） ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（会写 usage_records/daily_usage_rollups；用例使用随机 id）"]
    async fn overview_merges_finalized_rollup_and_details() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let provider_id = seed_provider(&state).await;
        let today = today_shanghai();
        let d1 = today - ChronoDuration::days(2);
        let d2 = today - ChronoDuration::days(1);
        cleanup_rollup_days(&state, &[d1, d2]).await;
        let prefix = Uuid::new_v4();
        let noon = |day: NaiveDate| {
            shanghai_midnight_utc(day).expect("midnight") + chrono::Duration::hours(12)
        };
        let now = Utc::now();
        for (request_id, provider, created_at, status) in [
            (
                format!("s51-ov-1-{prefix}"),
                Some(provider_id),
                noon(d1),
                UsageStatus::Success,
            ),
            (
                format!("s51-ov-2-{prefix}"),
                None,
                noon(d1),
                UsageStatus::Success,
            ),
            (
                format!("s51-ov-3-{prefix}"),
                Some(provider_id),
                noon(d1),
                UsageStatus::Rejected,
            ),
            (
                format!("s51-ov-4-{prefix}"),
                Some(provider_id),
                noon(d2),
                UsageStatus::Success,
            ),
            (
                format!("s51-ov-5-{prefix}"),
                Some(provider_id),
                now,
                UsageStatus::Success,
            ),
            (
                format!("s51-ov-6-{prefix}"),
                None,
                now,
                UsageStatus::UpstreamError,
            ),
        ] {
            seed_usage(
                &state,
                user_id,
                device_id,
                key_id,
                provider,
                created_at,
                &request_id,
                status,
            )
            .await;
        }
        assert!(finalize_day(&state, d1).await.expect("finalize d1"));
        assert!(finalize_day(&state, d2).await.expect("finalize d2"));

        let response = overview(&state, &OverviewQuery { days: Some(7) })
            .await
            .expect("overview");
        assert_eq!(response.trend.len(), 7);
        assert_eq!(response.trend.last().expect("today").day, today);

        let pool = state.db.as_ref().expect("pool");
        let finalized = finalized_day_set(&state).await;
        let first_day = today - ChronoDuration::days(6);
        let mut expected_totals = AggregateMetrics::default();
        for (offset, trend) in response.trend.iter().enumerate() {
            let day = first_day + ChronoDuration::days(offset as i64);
            let expected = if finalized.contains(&day) {
                sql_rollup_metrics(pool, day).await
            } else {
                let (start, end) = shanghai_day_utc_bounds(day).expect("bounds");
                sql_detail_metrics(pool, start, end).await
            };
            assert_eq!(trend.day, day);
            assert_metrics_eq(&trend.metrics, &expected);
            add_metrics(&mut expected_totals, &expected);
        }
        assert_metrics_eq(&response.totals, &expected_totals);

        let (today_start, today_end) = shanghai_day_utc_bounds(today).expect("today bounds");
        let today_expected = sql_detail_metrics(pool, today_start, today_end).await;
        assert_metrics_eq(&response.today.metrics, &today_expected);
        assert_eq!(
            response.today.status_breakdown["success"],
            serde_json::json!(today_expected.success_requests)
        );
        assert_eq!(
            response.today.status_breakdown["upstream_error"],
            serde_json::json!(today_expected.error_requests)
        );

        cleanup_rollup_days(&state, &[d1, d2]).await;
        cleanup_users_and_usage(&state, &[user_id], &[provider_id]).await;
    }

    // ---------- S5.1 gated：usage 合并 + 分组/过滤/分页 + 清理后读 rollup ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（会写 usage_records/daily_usage_rollups；用例使用随机 id）"]
    async fn usage_merges_rollup_and_details_with_filters_and_pagination() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let provider_id = seed_provider(&state).await;
        let today = today_shanghai();
        let d1 = today - ChronoDuration::days(2);
        let d2 = today - ChronoDuration::days(1);
        cleanup_rollup_days(&state, &[d1, d2]).await;
        let prefix = Uuid::new_v4();
        let noon = |day: NaiveDate| {
            shanghai_midnight_utc(day).expect("midnight") + chrono::Duration::hours(12)
        };
        let now = Utc::now();
        for (request_id, provider, created_at, status) in [
            (
                format!("s51-us-1-{prefix}"),
                Some(provider_id),
                noon(d1),
                UsageStatus::Success,
            ),
            (
                format!("s51-us-2-{prefix}"),
                None,
                noon(d1),
                UsageStatus::Rejected,
            ),
            (
                format!("s51-us-3-{prefix}"),
                Some(provider_id),
                noon(d2),
                UsageStatus::Success,
            ),
            (
                format!("s51-us-4-{prefix}"),
                Some(provider_id),
                now,
                UsageStatus::Success,
            ),
            (
                format!("s51-us-5-{prefix}"),
                None,
                now,
                UsageStatus::UpstreamError,
            ),
        ] {
            seed_usage(
                &state,
                user_id,
                device_id,
                key_id,
                provider,
                created_at,
                &request_id,
                status,
            )
            .await;
        }
        assert!(finalize_day(&state, d1).await.expect("finalize d1"));
        assert!(finalize_day(&state, d2).await.expect("finalize d2"));

        let from = shanghai_midnight_utc(d1).expect("from");
        let to = now + chrono::Duration::hours(1);
        let base_query = UsageQuery {
            from: Some(from),
            to: Some(to),
            group_by: Some("day,model_alias,key_id".to_string()),
            user_id: Some(user_id),
            key_id: None,
            provider_id: None,
            model_alias: None,
            limit: Some(50),
            offset: Some(0),
        };
        let response = usage(&state, &base_query).await.expect("usage merged");
        let pool = state.db.as_ref().expect("pool");
        let expected = sql_usage_groups(pool, from, to, user_id, None).await;
        assert_eq!(
            i64::try_from(expected.len()).expect("len"),
            response.total,
            "merged total"
        );
        assert_eq!(response.items.len(), expected.len());
        assert_usage_items_match(&response.items, &expected);
        let expected_totals = sum_expected_groups(&expected);
        assert_metrics_eq(&response.totals, &expected_totals);

        // provider_id 过滤（rollup provider 维度）。
        let provider_query = UsageQuery {
            provider_id: Some(provider_id),
            ..base_query.clone()
        };
        let provider_response = usage(&state, &provider_query)
            .await
            .expect("usage provider");
        let provider_expected = sql_usage_groups(pool, from, to, user_id, Some(provider_id)).await;
        assert_eq!(provider_response.items.len(), provider_expected.len());
        assert_usage_items_match(&provider_response.items, &provider_expected);
        assert_metrics_eq(
            &provider_response.totals,
            &sum_expected_groups(&provider_expected),
        );

        // model_alias 过滤 + 非 day 分组。
        let alias_query = UsageQuery {
            group_by: Some("model_alias".to_string()),
            model_alias: Some("fast".to_string()),
            ..base_query.clone()
        };
        let alias_response = usage(&state, &alias_query).await.expect("usage alias");
        assert_eq!(alias_response.items.len(), 1);
        assert_eq!(alias_response.items[0].model_alias.as_deref(), Some("fast"));
        assert!(alias_response.items[0].day.is_none());

        // 分页：合并后排序/limit/offset。
        let page_query = UsageQuery {
            limit: Some(1),
            offset: Some(1),
            ..base_query.clone()
        };
        let page_response = usage(&state, &page_query).await.expect("usage page");
        assert_eq!(page_response.total, response.total);
        assert_eq!(page_response.items.len(), 1);
        let expected_page = &expected[1..2];
        assert_usage_items_match(&page_response.items, expected_page);

        // 模拟明细保留清理：删除已 finalized 日的本用户明细，历史仍从 rollup 读。
        let (d1_start, _) = shanghai_day_utc_bounds(d1).expect("d1 bounds");
        let (d2_end, _) = shanghai_day_utc_bounds(d2 + ChronoDuration::days(1)).expect("d2 end");
        sqlx::query(
            "DELETE FROM usage_records \
             WHERE user_id = $1 AND created_at >= $2 AND created_at < $3",
        )
        .bind(user_id)
        .bind(d1_start)
        .bind(d2_end)
        .execute(pool)
        .await
        .expect("delete finalized details");
        let after = usage(&state, &base_query)
            .await
            .expect("usage after cleanup");
        assert_eq!(after.total, response.total);
        assert_eq!(after.items.len(), response.items.len());
        assert_usage_items_match(&after.items, &expected);
        assert_metrics_eq(&after.totals, &expected_totals);

        cleanup_rollup_days(&state, &[d1, d2]).await;
        cleanup_users_and_usage(&state, &[user_id], &[provider_id]).await;
    }

    #[derive(Debug, Clone)]
    struct ExpectedUsageGroup {
        day: NaiveDate,
        model_alias: String,
        key_id: Uuid,
        metrics: AggregateMetrics,
    }

    async fn sql_usage_groups(
        pool: &sqlx::PgPool,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        user_id: Uuid,
        provider_id: Option<Uuid>,
    ) -> Vec<ExpectedUsageGroup> {
        let rows = sqlx::query(
            "SELECT (created_at AT TIME ZONE 'Asia/Shanghai')::date AS day, \
                 model_alias, key_id, \
                 count(*) AS requests, \
                 count(*) FILTER (WHERE status = 'success') AS success_requests, \
                 count(*) FILTER (WHERE status IN ('upstream_error', 'timeout', 'gateway_error')) AS error_requests, \
                 count(*) FILTER (WHERE status = 'rejected') AS rejected_requests, \
                 count(*) FILTER (WHERE status = 'client_disconnect') AS client_disconnect_requests, \
                 coalesce(sum(input_tokens), 0)::bigint AS input_tokens, \
                 coalesce(sum(output_tokens), 0)::bigint AS output_tokens, \
                 coalesce(sum(cost), 0)::text AS cost \
             FROM usage_records \
             WHERE created_at >= $1 AND created_at < $2 AND user_id = $3 \
               AND ($4::uuid IS NULL OR provider_id = $4) \
             GROUP BY 1, 2, 3 ORDER BY 1, 2, 3",
        )
        .bind(start)
        .bind(end)
        .bind(user_id)
        .bind(provider_id)
        .fetch_all(pool)
        .await
        .expect("usage groups");
        rows.iter()
            .map(|row| ExpectedUsageGroup {
                day: row.get("day"),
                model_alias: row.get("model_alias"),
                key_id: row.get("key_id"),
                metrics: metrics_from_row(row),
            })
            .collect()
    }

    fn sum_expected_groups(groups: &[ExpectedUsageGroup]) -> AggregateMetrics {
        let mut totals = AggregateMetrics::default();
        for group in groups {
            add_metrics(&mut totals, &group.metrics);
        }
        totals
    }

    fn assert_usage_items_match(items: &[UsageItem], expected: &[ExpectedUsageGroup]) {
        assert_eq!(items.len(), expected.len());
        for (item, group) in items.iter().zip(expected) {
            assert_eq!(item.day, Some(group.day));
            assert_eq!(
                item.model_alias.as_deref(),
                Some(group.model_alias.as_str())
            );
            assert_eq!(item.key_id, Some(group.key_id));
            assert_metrics_eq(&item.metrics, &group.metrics);
        }
    }

    // ---------- S5.1 gated：维护任务（日结 → 清理 → rollup 保留） ----------

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（会写 usage_records/daily_usage_rollups；用例使用随机 id）"]
    async fn run_maintenance_once_finalizes_cleans_and_retains_rollup() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let (user_id, device_id, key_id) = seed_identity(&state).await;
        let today = today_shanghai();
        let old_day = today - ChronoDuration::days(200);
        let unfinalized_old_day = today - ChronoDuration::days(190);
        let expired_rollup_day = today - ChronoDuration::days(2000);
        cleanup_rollup_days(&state, &[old_day, unfinalized_old_day, expired_rollup_day]).await;
        let before = finalized_day_set(&state).await;

        let prefix = Uuid::new_v4();
        let office = |day: NaiveDate, hour: i64| {
            shanghai_midnight_utc(day).expect("midnight") + chrono::Duration::hours(hour)
        };
        for (name, hour) in [("maint-1", 9), ("maint-2", 10)] {
            seed_usage(
                &state,
                user_id,
                device_id,
                key_id,
                None,
                office(old_day, hour),
                &format!("s51-{name}-{prefix}"),
                UsageStatus::Success,
            )
            .await;
        }
        let today_request = format!("s51-maint-today-{prefix}");
        seed_usage(
            &state,
            user_id,
            device_id,
            key_id,
            None,
            Utc::now(),
            &today_request,
            UsageStatus::Success,
        )
        .await;
        // 过期的 finalized rollup（超过 rollup 保留期）应被清理。
        let pool = state.db.as_ref().expect("pool");
        sqlx::query(
            "INSERT INTO daily_usage_rollups \
                 (day, user_id, key_id, model_alias, provider_id, requests, success_requests, \
                  error_requests, rejected_requests, client_disconnect_requests, \
                  input_tokens, output_tokens, cost, computed_at) \
             VALUES ($1, $2, $3, 'synthetic', $4, 0, 0, 0, 0, 0, 0, 0, 0, now()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(expired_rollup_day)
        .bind(user_id)
        .bind(key_id)
        .bind(ZERO_PROVIDER_ID)
        .execute(pool)
        .await
        .expect("insert synthetic rollup");
        sqlx::query(
            "INSERT INTO usage_rollup_days \
                 (day, finalized, computed_at, source_records, source_cost, rollup_records, rollup_cost) \
             VALUES ($1, true, now(), 0, 0, 0, 0) \
             ON CONFLICT (day) DO UPDATE SET finalized = true",
        )
        .bind(expired_rollup_day)
        .execute(pool)
        .await
        .expect("insert synthetic rollup day");

        assert_eq!(count_user_usage_in_day(pool, user_id, old_day).await, 2);
        run_maintenance_once(&state).await;

        // 过期日已日结：rollup 行存在（保留期内），明细被清理。
        assert!(finalized_day_set(&state).await.contains(&old_day));
        let old_rollup: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM daily_usage_rollups WHERE day = $1 AND user_id = $2",
        )
        .bind(old_day)
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("old rollup count");
        assert!(old_rollup >= 1);
        assert_eq!(count_user_usage_in_day(pool, user_id, old_day).await, 0);
        // 今天不结账、明细保留。
        assert_eq!(
            count_user_usage_in_day(pool, user_id, today).await,
            1,
            "today detail must be retained"
        );
        // 超过 rollup 保留期的聚合/元数据被清理。
        let expired_rollups: i64 =
            sqlx::query_scalar("SELECT count(*) FROM daily_usage_rollups WHERE day = $1")
                .bind(expired_rollup_day)
                .fetch_one(pool)
                .await
                .expect("expired rollup count");
        assert_eq!(expired_rollups, 0);
        let expired_days: i64 =
            sqlx::query_scalar("SELECT count(*) FROM usage_rollup_days WHERE day = $1")
                .bind(expired_rollup_day)
                .fetch_one(pool)
                .await
                .expect("expired rollup day count");
        assert_eq!(expired_days, 0);

        // 未 finalized 的过期明细不会被清理（直接调用清理步骤）。
        seed_usage(
            &state,
            user_id,
            device_id,
            key_id,
            None,
            office(unfinalized_old_day, 9),
            &format!("s51-maint-unfinalized-{prefix}"),
            UsageStatus::Success,
        )
        .await;
        let deleted = cleanup_expired_details(&state).await.expect("cleanup step");
        assert_eq!(deleted, 0, "unfinalized day must not be cleaned");
        assert_eq!(
            count_user_usage_in_day(pool, user_id, unfinalized_old_day).await,
            1
        );

        let after = finalized_day_set(&state).await;
        let mut new_days: Vec<NaiveDate> = after.difference(&before).copied().collect();
        new_days.push(unfinalized_old_day);
        cleanup_rollup_days(&state, &new_days).await;
        cleanup_users_and_usage(&state, &[user_id], &[]).await;
    }
}
