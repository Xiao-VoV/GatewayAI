//! observability 数据访问（表归属：`request_logs`/`audit_logs`）。
//!
//! - 请求日志写入幂等：`INSERT ... ON CONFLICT (request_id) DO NOTHING`；
//! - 审计只追加：单条 INSERT，无 UPDATE/DELETE 业务路径（保留期清理除外）；
//! - 列表/导出 COUNT 与数据在同一 REPEATABLE READ 只读事务内，保证 total 与行一致；
//! - 只写元数据列，**绝无**正文/Authorization/Key/token/Cookie。

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use sqlx::postgres::PgArguments;
use sqlx::query::{QueryAs, QueryScalar};
use uuid::Uuid;

use crate::platform::error::AppError;

/// 一行 `request_logs` 的最终写入值（已 sanitize）。
#[derive(Debug)]
pub struct NewRequestLog {
    pub id: Uuid,
    pub request_id: String,
    pub trace_id: Option<String>,
    pub user_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
    pub key_id: Option<Uuid>,
    pub key_prefix: Option<String>,
    pub method: String,
    pub path: String,
    pub protocol: &'static str,
    pub upstream_protocol: Option<String>,
    pub adapter: Option<String>,
    pub model_alias: Option<String>,
    pub upstream_model: Option<String>,
    pub provider: Option<String>,
    pub provider_id: Option<Uuid>,
    pub is_stream: bool,
    pub fallback: bool,
    pub attempt_count: i32,
    pub status_code: Option<i32>,
    pub latency_ms: i32,
    pub ttft_ms: Option<i32>,
    pub response_size: Option<i64>,
    pub error_code: Option<String>,
    pub client_ip: Option<String>,
    pub user_agent: String,
    pub created_at: DateTime<Utc>,
}

/// 幂等写入一条请求日志；`request_id` 冲突时静默忽略（返回 Ok）。
pub async fn insert_request_log(pool: &PgPool, record: &NewRequestLog) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO request_logs (\
             id, request_id, trace_id, user_id, device_id, key_id, key_prefix, \
             method, path, protocol, upstream_protocol, adapter, \
             model_alias, upstream_model, provider, provider_id, \
             is_stream, fallback, attempt_count, \
             status_code, latency_ms, ttft_ms, response_size, error_code, \
             client_ip, user_agent, created_at\
         ) VALUES (\
             $1, $2, $3, $4, $5, $6, $7, \
             $8, $9, $10, $11, $12, \
             $13, $14, $15, $16, \
             $17, $18, $19, \
             $20, $21, $22, $23, $24, \
             $25, $26, $27\
         ) ON CONFLICT (request_id) DO NOTHING",
    )
    .bind(record.id)
    .bind(&record.request_id)
    .bind(record.trace_id.as_deref())
    .bind(record.user_id)
    .bind(record.device_id)
    .bind(record.key_id)
    .bind(record.key_prefix.as_deref())
    .bind(&record.method)
    .bind(&record.path)
    .bind(record.protocol)
    .bind(record.upstream_protocol.as_deref())
    .bind(record.adapter.as_deref())
    .bind(record.model_alias.as_deref())
    .bind(record.upstream_model.as_deref())
    .bind(record.provider.as_deref())
    .bind(record.provider_id)
    .bind(record.is_stream)
    .bind(record.fallback)
    .bind(record.attempt_count)
    .bind(record.status_code)
    .bind(record.latency_ms)
    .bind(record.ttft_ms)
    .bind(record.response_size)
    .bind(record.error_code.as_deref())
    .bind(record.client_ip.as_deref())
    .bind(&record.user_agent)
    .bind(record.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- S5 管理查询 ----------

#[derive(Debug, Clone)]
pub struct RequestFilters {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub user_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
    pub key_id: Option<Uuid>,
    pub key_prefix: Option<String>,
    pub model_alias: Option<String>,
    pub provider: Option<String>,
    pub status_min: Option<i32>,
    pub status_max: Option<i32>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RequestLogRow {
    pub id: Uuid,
    pub request_id: String,
    pub trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub protocol: String,
    pub upstream_protocol: Option<String>,
    pub adapter: Option<String>,
    pub user_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
    pub key_id: Option<Uuid>,
    pub key_prefix: Option<String>,
    pub model_alias: Option<String>,
    pub upstream_model: Option<String>,
    pub provider: Option<String>,
    pub provider_id: Option<Uuid>,
    pub is_stream: bool,
    pub fallback: bool,
    pub attempt_count: i32,
    pub status_code: Option<i32>,
    pub latency_ms: i32,
    pub ttft_ms: Option<i32>,
    pub response_size: Option<i64>,
    pub error_code: Option<String>,
    pub client_ip: Option<String>,
    pub user_agent: String,
}

const REQUEST_LOG_COLUMNS: &str = "id, request_id, trace_id, created_at, method, path, protocol, \
    upstream_protocol, adapter, user_id, device_id, key_id, key_prefix, model_alias, upstream_model, \
    provider, provider_id, is_stream, fallback, attempt_count, status_code, latency_ms, ttft_ms, \
    response_size, error_code, client_ip, user_agent";

const REQUEST_FILTER_SQL: &str = "created_at >= $1 AND created_at < $2 \
    AND ($3::uuid IS NULL OR user_id = $3) \
    AND ($4::uuid IS NULL OR device_id = $4) \
    AND ($5::uuid IS NULL OR key_id = $5) \
    AND ($6::text IS NULL OR key_prefix = $6) \
    AND ($7::text IS NULL OR model_alias = $7) \
    AND ($8::text IS NULL OR provider = $8) \
    AND ($9::int IS NULL OR status_code >= $9) \
    AND ($10::int IS NULL OR status_code <= $10) \
    AND ($11::text IS NULL OR trace_id = $11 OR request_id = $11)";

fn bind_request_filters_as<'q, O>(
    query: QueryAs<'q, sqlx::Postgres, O, PgArguments>,
    filters: &'q RequestFilters,
) -> QueryAs<'q, sqlx::Postgres, O, PgArguments> {
    query
        .bind(filters.from)
        .bind(filters.to)
        .bind(filters.user_id)
        .bind(filters.device_id)
        .bind(filters.key_id)
        .bind(filters.key_prefix.as_deref())
        .bind(filters.model_alias.as_deref())
        .bind(filters.provider.as_deref())
        .bind(filters.status_min)
        .bind(filters.status_max)
        .bind(filters.trace_id.as_deref())
}

fn bind_request_filters_scalar<'q, O>(
    query: QueryScalar<'q, sqlx::Postgres, O, PgArguments>,
    filters: &'q RequestFilters,
) -> QueryScalar<'q, sqlx::Postgres, O, PgArguments> {
    query
        .bind(filters.from)
        .bind(filters.to)
        .bind(filters.user_id)
        .bind(filters.device_id)
        .bind(filters.key_id)
        .bind(filters.key_prefix.as_deref())
        .bind(filters.model_alias.as_deref())
        .bind(filters.provider.as_deref())
        .bind(filters.status_min)
        .bind(filters.status_max)
        .bind(filters.trace_id.as_deref())
}

/// 列表 + COUNT 在同一 REPEATABLE READ 只读事务内执行，保证 total 与 items 同快照。
pub async fn list_request_logs(
    pool: &PgPool,
    filters: &RequestFilters,
    limit: i64,
    offset: i64,
) -> Result<(Vec<RequestLogRow>, i64), AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let total: i64 = bind_request_filters_scalar(
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM request_logs WHERE {REQUEST_FILTER_SQL}"
        )),
        filters,
    )
    .fetch_one(&mut *tx)
    .await?;
    let rows = bind_request_filters_as(
        sqlx::query_as::<_, RequestLogRow>(&format!(
            "SELECT {REQUEST_LOG_COLUMNS} FROM request_logs WHERE {REQUEST_FILTER_SQL} \
             ORDER BY created_at DESC, id DESC LIMIT $12 OFFSET $13"
        )),
        filters,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((rows, total))
}

/// CSV 导出请求日志（同列表筛选；超过 `max_rows` 返回 400 `export_too_large`，不截断）。
pub async fn export_request_logs(
    pool: &PgPool,
    filters: &RequestFilters,
    max_rows: i64,
) -> Result<Vec<RequestLogRow>, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let total: i64 = bind_request_filters_scalar(
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM request_logs WHERE {REQUEST_FILTER_SQL}"
        )),
        filters,
    )
    .fetch_one(&mut *tx)
    .await?;
    if total > max_rows {
        return Err(export_too_large(total, max_rows));
    }
    let rows = bind_request_filters_as(
        sqlx::query_as::<_, RequestLogRow>(&format!(
            "SELECT {REQUEST_LOG_COLUMNS} FROM request_logs WHERE {REQUEST_FILTER_SQL} \
             ORDER BY created_at DESC, id DESC LIMIT $12"
        )),
        filters,
    )
    .bind(max_rows)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

fn export_too_large(total: i64, max_rows: i64) -> AppError {
    AppError::bad_request(
        "export_too_large",
        format!("导出行数 {total} 超过上限 {max_rows}，请缩小时间范围或筛选条件"),
    )
}

pub async fn get_request_log(
    pool: &PgPool,
    request_id: &str,
) -> Result<Option<RequestLogRow>, AppError> {
    let row = sqlx::query_as::<_, RequestLogRow>(&format!(
        "SELECT {REQUEST_LOG_COLUMNS} FROM request_logs WHERE request_id = $1"
    ))
    .bind(request_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// S5.1 审计（audit_logs 只追加）
// ---------------------------------------------------------------------------

/// 一行审计的最终写入值（detail 已 sanitize）。
#[derive(Debug)]
pub struct NewAuditLog {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub actor_type: String,
    pub actor_id: Option<Uuid>,
    pub actor_email: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub outcome: String,
    pub client_ip: Option<String>,
    pub user_agent: String,
    pub request_id: Option<String>,
    pub detail: Value,
}

/// 只追加一条审计记录（无幂等键；审计事件不重试重复写由 detached 有界重试保证收敛）。
pub async fn insert_audit_log(pool: &PgPool, record: &NewAuditLog) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO audit_logs (\
             id, created_at, actor_type, actor_id, actor_email, action, \
             resource_type, resource_id, outcome, client_ip, user_agent, request_id, detail\
         ) VALUES (\
             $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13\
         )",
    )
    .bind(record.id)
    .bind(record.created_at)
    .bind(&record.actor_type)
    .bind(record.actor_id)
    .bind(record.actor_email.as_deref())
    .bind(&record.action)
    .bind(&record.resource_type)
    .bind(record.resource_id.as_deref())
    .bind(&record.outcome)
    .bind(record.client_ip.as_deref())
    .bind(&record.user_agent)
    .bind(record.request_id.as_deref())
    .bind(&record.detail)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AuditFilters {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub actor_id: Option<Uuid>,
    pub action: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuditLogRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub actor_type: String,
    pub actor_id: Option<Uuid>,
    pub actor_email: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub outcome: String,
    pub client_ip: Option<String>,
    pub user_agent: String,
    pub request_id: Option<String>,
    pub detail: Value,
}

const AUDIT_LOG_COLUMNS: &str = "id, created_at, actor_type, actor_id, actor_email, action, \
    resource_type, resource_id, outcome, client_ip, user_agent, request_id, detail";

const AUDIT_FILTER_SQL: &str = "created_at >= $1 AND created_at < $2 \
    AND ($3::uuid IS NULL OR actor_id = $3) \
    AND ($4::text IS NULL OR action = $4) \
    AND ($5::text IS NULL OR resource_type = $5) \
    AND ($6::text IS NULL OR resource_id = $6) \
    AND ($7::text IS NULL OR outcome = $7)";

fn bind_audit_filters_as<'q, O>(
    query: QueryAs<'q, sqlx::Postgres, O, PgArguments>,
    filters: &'q AuditFilters,
) -> QueryAs<'q, sqlx::Postgres, O, PgArguments> {
    query
        .bind(filters.from)
        .bind(filters.to)
        .bind(filters.actor_id)
        .bind(filters.action.as_deref())
        .bind(filters.resource_type.as_deref())
        .bind(filters.resource_id.as_deref())
        .bind(filters.outcome.as_deref())
}

fn bind_audit_filters_scalar<'q, O>(
    query: QueryScalar<'q, sqlx::Postgres, O, PgArguments>,
    filters: &'q AuditFilters,
) -> QueryScalar<'q, sqlx::Postgres, O, PgArguments> {
    query
        .bind(filters.from)
        .bind(filters.to)
        .bind(filters.actor_id)
        .bind(filters.action.as_deref())
        .bind(filters.resource_type.as_deref())
        .bind(filters.resource_id.as_deref())
        .bind(filters.outcome.as_deref())
}

/// 审计列表 + COUNT（REPEATABLE READ READ ONLY，同快照；latest first）。
pub async fn list_audit_logs(
    pool: &PgPool,
    filters: &AuditFilters,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AuditLogRow>, i64), AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let total: i64 = bind_audit_filters_scalar(
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM audit_logs WHERE {AUDIT_FILTER_SQL}"
        )),
        filters,
    )
    .fetch_one(&mut *tx)
    .await?;
    let rows = bind_audit_filters_as(
        sqlx::query_as::<_, AuditLogRow>(&format!(
            "SELECT {AUDIT_LOG_COLUMNS} FROM audit_logs WHERE {AUDIT_FILTER_SQL} \
             ORDER BY created_at DESC, id DESC LIMIT $8 OFFSET $9"
        )),
        filters,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((rows, total))
}

/// CSV 导出审计（同查询筛选；超过 `max_rows` 返回 400 `export_too_large`，不截断）。
pub async fn export_audit_logs(
    pool: &PgPool,
    filters: &AuditFilters,
    max_rows: i64,
) -> Result<Vec<AuditLogRow>, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let total: i64 = bind_audit_filters_scalar(
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM audit_logs WHERE {AUDIT_FILTER_SQL}"
        )),
        filters,
    )
    .fetch_one(&mut *tx)
    .await?;
    if total > max_rows {
        return Err(export_too_large(total, max_rows));
    }
    let rows = bind_audit_filters_as(
        sqlx::query_as::<_, AuditLogRow>(&format!(
            "SELECT {AUDIT_LOG_COLUMNS} FROM audit_logs WHERE {AUDIT_FILTER_SQL} \
             ORDER BY created_at DESC, id DESC LIMIT $8"
        )),
        filters,
    )
    .bind(max_rows)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// S5.1 保留清理：请求日志/审计按保留期分批删除（ctid 子查询，避免长锁）
// ---------------------------------------------------------------------------

/// 删除一批过期请求日志（保留期清理是 request_logs 的唯一删除路径）。
pub async fn delete_expired_request_logs_batch(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    batch_size: i64,
) -> Result<u64, AppError> {
    let result = sqlx::query(
        "WITH batch AS ( \
             SELECT ctid FROM request_logs WHERE created_at < $1 LIMIT $2 \
         ) \
         DELETE FROM request_logs r USING batch WHERE r.ctid = batch.ctid",
    )
    .bind(cutoff)
    .bind(batch_size)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// 删除一批过期审计（保留期清理是 audit_logs 的唯一合法删除路径，FR-30）。
pub async fn delete_expired_audit_logs_batch(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    batch_size: i64,
) -> Result<u64, AppError> {
    let result = sqlx::query(
        "WITH batch AS ( \
             SELECT ctid FROM audit_logs WHERE created_at < $1 LIMIT $2 \
         ) \
         DELETE FROM audit_logs a USING batch WHERE a.ctid = batch.ctid",
    )
    .bind(cutoff)
    .bind(batch_size)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
