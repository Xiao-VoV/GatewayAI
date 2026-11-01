//! metering 数据访问（表归属：`usage_records`/`daily_usage_rollups`/`usage_rollup_days`）。
//!
//! - 写入幂等：`INSERT ... ON CONFLICT (request_id) DO NOTHING`；
//! - 金额按 `::text::numeric` 绑定（`rust_decimal` 未开启 sqlx feature，避免驱动浮点）；
//! - 日结/维护 SQL 供 service 在事务内调用；读路径 SQL 由 service 组合事务与合并。

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use rust_decimal::Decimal;
use sqlx::postgres::{PgArguments, PgRow};
use sqlx::query::{Query, QueryScalar};
use sqlx::{PgConnection, PgPool, Row};
use std::str::FromStr;
use uuid::Uuid;

use crate::platform::error::AppError;

/// rollup 中“无供应商”哨兵（与迁移 0010 的 default 一致；非 FK）。
pub const ZERO_PROVIDER_ID: Uuid = Uuid::nil();

/// 日结 advisory lock 命名空间（事务级；多实例重复日结的串行化预留）。
const ROLLUP_LOCK_NAMESPACE: i32 = 42_007;

// ---------------------------------------------------------------------------
// 写入路径（proxy → metering）
// ---------------------------------------------------------------------------

/// 一行 `usage_records` 的最终写入值（成本已由 service 计算）。
#[derive(Debug)]
pub struct NewUsageRecord {
    pub id: Uuid,
    pub request_id: String,
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub key_id: Uuid,
    pub model_alias: String,
    pub deployment_id: Option<Uuid>,
    pub source: &'static str,
    pub resolved_upstream_model: String,
    pub provider_id: Option<Uuid>,
    pub provider_name: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub usage_source: &'static str,
    pub cost: Decimal,
    pub currency: &'static str,
    pub unit_price_input: Option<Decimal>,
    pub unit_price_output: Option<Decimal>,
    pub price_currency: &'static str,
    pub fx_rate: Option<Decimal>,
    pub latency_ms: i32,
    pub ttft_ms: Option<i32>,
    pub is_stream: bool,
    pub fallback: bool,
    pub attempt_count: i32,
    pub status: &'static str,
    pub http_status: Option<i32>,
    pub error_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// 幂等写入一条用量明细；`request_id` 冲突时静默忽略（返回 Ok）。
pub async fn insert_usage(pool: &PgPool, record: &NewUsageRecord) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO usage_records (\
             id, request_id, user_id, device_id, key_id, model_alias, deployment_id, source, \
             resolved_upstream_model, provider_id, provider_name, \
             input_tokens, output_tokens, usage_source, \
             cost, currency, unit_price_input, unit_price_output, price_currency, fx_rate, \
             latency_ms, ttft_ms, is_stream, fallback, attempt_count, \
             status, http_status, error_code, created_at\
         ) VALUES (\
             $1, $2, $3, $4, $5, $6, $7, $8, \
             $9, $10, $11, \
             $12, $13, $14, \
             $15::text::numeric, $16, $17::text::numeric, $18::text::numeric, $19, \
             $20::text::numeric, \
             $21, $22, $23, $24, $25, \
             $26, $27, $28, $29\
         ) ON CONFLICT (request_id) DO NOTHING",
    )
    .bind(record.id)
    .bind(&record.request_id)
    .bind(record.user_id)
    .bind(record.device_id)
    .bind(record.key_id)
    .bind(&record.model_alias)
    .bind(record.deployment_id)
    .bind(record.source)
    .bind(&record.resolved_upstream_model)
    .bind(record.provider_id)
    .bind(&record.provider_name)
    .bind(record.input_tokens)
    .bind(record.output_tokens)
    .bind(record.usage_source)
    .bind(decimal_text(record.cost))
    .bind(record.currency)
    .bind(record.unit_price_input.map(decimal_text))
    .bind(record.unit_price_output.map(decimal_text))
    .bind(record.price_currency)
    .bind(record.fx_rate.map(decimal_text))
    .bind(record.latency_ms)
    .bind(record.ttft_ms)
    .bind(record.is_stream)
    .bind(record.fallback)
    .bind(record.attempt_count)
    .bind(record.status)
    .bind(record.http_status)
    .bind(record.error_code.as_deref())
    .bind(record.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// 聚合 `[start, end)` 内某用户的成本（CNY，6 位小数）。
///
/// 口径与 policy 预算一致：Asia/Shanghai 当日、`created_at` 为请求开始时间；
/// 当日全部状态都计入（rejected/upstream_error 的 cost=0，不影响金额）。
pub async fn sum_user_cost(
    pool: &PgPool,
    user_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Decimal, AppError> {
    let spent: String = sqlx::query_scalar(
        "SELECT coalesce(sum(cost), 0)::text \
         FROM usage_records \
         WHERE user_id = $1 AND created_at >= $2 AND created_at < $3",
    )
    .bind(user_id)
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await?;
    decimal_from_text(&spent)
}

/// `NUMERIC` 的文本绑定形式（避免 f64；与 policy repo 的 `::text::numeric` 一致）。
fn decimal_text(value: Decimal) -> String {
    value.to_string()
}

fn decimal_from_text(raw: &str) -> Result<Decimal, AppError> {
    Decimal::from_str(raw).map_err(|error| {
        tracing::error!(%error, value = %raw, "invalid numeric from metering aggregate");
        AppError::internal("invalid metering numeric aggregate")
    })
}

// ---------------------------------------------------------------------------
// 日结（S5.1）：finalize_day 事务内的 SQL
// ---------------------------------------------------------------------------

/// 事务级 advisory lock：同一 day 的日结串行化（单实例 S5 仅防御重复 tick）。
pub async fn lock_rollup_day(conn: &mut PgConnection, day: NaiveDate) -> Result<(), AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock($1::int4, $2::int4)")
        .bind(ROLLUP_LOCK_NAMESPACE)
        .bind(day.num_days_from_ce())
        .execute(conn)
        .await?;
    Ok(())
}

/// 删除某日 rollup（整日全量重算前置；失败回滚会恢复旧聚合）。
pub async fn delete_rollup_day(conn: &mut PgConnection, day: NaiveDate) -> Result<(), AppError> {
    sqlx::query("DELETE FROM daily_usage_rollups WHERE day = $1")
        .bind(day)
        .execute(conn)
        .await?;
    Ok(())
}

/// 从 `usage_records` 聚合整日并写入 `daily_usage_rollups`。
///
/// 只统计已鉴权请求（`user_id`/`key_id` 非空，rollup 列为 NOT NULL）；
/// `provider_id` NULL → zero UUID 哨兵（0007 的 provider 快照可被 SET NULL）。
pub async fn insert_rollup_day(
    conn: &mut PgConnection,
    day: NaiveDate,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO daily_usage_rollups (\
             day, user_id, key_id, model_alias, provider_id, \
             requests, success_requests, error_requests, rejected_requests, \
             client_disconnect_requests, input_tokens, output_tokens, cost, computed_at\
         ) \
         SELECT $1, user_id, key_id, model_alias, coalesce(provider_id, $2::uuid), \
             count(*)::bigint, \
             count(*) FILTER (WHERE status = 'success')::bigint, \
             count(*) FILTER (WHERE status IN ('upstream_error', 'timeout', 'gateway_error'))::bigint, \
             count(*) FILTER (WHERE status = 'rejected')::bigint, \
             count(*) FILTER (WHERE status = 'client_disconnect')::bigint, \
             coalesce(sum(input_tokens), 0)::bigint, \
             coalesce(sum(output_tokens), 0)::bigint, \
             coalesce(sum(cost), 0), \
             now() \
         FROM usage_records \
         WHERE created_at >= $3 AND created_at < $4 \
           AND user_id IS NOT NULL AND key_id IS NOT NULL \
         GROUP BY user_id, key_id, model_alias, provider_id",
    )
    .bind(day)
    .bind(ZERO_PROVIDER_ID)
    .bind(start)
    .bind(end)
    .execute(conn)
    .await?;
    Ok(())
}

/// 日结输入侧合计：明细行数与成本（用于 `usage_rollup_days.source_*` 对账证据）。
pub async fn source_totals(
    conn: &mut PgConnection,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<(i64, Decimal), AppError> {
    let (records, cost): (i64, String) = sqlx::query_as(
        "SELECT count(*)::bigint AS records, coalesce(sum(cost), 0)::text AS cost \
         FROM usage_records \
         WHERE created_at >= $1 AND created_at < $2 \
           AND user_id IS NOT NULL AND key_id IS NOT NULL",
    )
    .bind(start)
    .bind(end)
    .fetch_one(conn)
    .await?;
    Ok((records, decimal_from_text(&cost)?))
}

/// 日结输出侧合计：本次写入 rollup 的请求数与成本（用于平衡校验）。
pub async fn rollup_totals(
    conn: &mut PgConnection,
    day: NaiveDate,
) -> Result<(i64, Decimal), AppError> {
    let (records, cost): (i64, String) = sqlx::query_as(
        "SELECT coalesce(sum(requests), 0)::bigint AS records, \
                coalesce(sum(cost), 0)::text AS cost \
         FROM daily_usage_rollups WHERE day = $1",
    )
    .bind(day)
    .fetch_one(conn)
    .await?;
    Ok((records, decimal_from_text(&cost)?))
}

/// UPSERT 日结元数据（finalized=true；DB CHECK 兜底 source/rollup 平衡）。
pub async fn upsert_rollup_day(
    conn: &mut PgConnection,
    day: NaiveDate,
    source_records: i64,
    source_cost: Decimal,
    rollup_records: i64,
    rollup_cost: Decimal,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO usage_rollup_days \
             (day, finalized, computed_at, source_records, source_cost, rollup_records, rollup_cost) \
         VALUES ($1, true, now(), $2, $3::text::numeric, $4, $5::text::numeric) \
         ON CONFLICT (day) DO UPDATE SET \
             finalized = EXCLUDED.finalized, computed_at = EXCLUDED.computed_at, \
             source_records = EXCLUDED.source_records, source_cost = EXCLUDED.source_cost, \
             rollup_records = EXCLUDED.rollup_records, rollup_cost = EXCLUDED.rollup_cost",
    )
    .bind(day)
    .bind(source_records)
    .bind(decimal_text(source_cost))
    .bind(rollup_records)
    .bind(decimal_text(rollup_cost))
    .execute(conn)
    .await?;
    Ok(())
}

/// 待日结日（`< today` 且未 finalized）：有明细或存在未完成日结元数据；最多 `limit` 天。
pub async fn due_finalize_days(
    pool: &PgPool,
    today: NaiveDate,
    today_start: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<NaiveDate>, AppError> {
    let days = sqlx::query_scalar::<_, NaiveDate>(
        "SELECT day FROM ( \
             SELECT (created_at AT TIME ZONE 'Asia/Shanghai')::date AS day \
             FROM usage_records WHERE created_at < $2 GROUP BY 1 \
             UNION \
             SELECT day FROM usage_rollup_days WHERE finalized = false \
         ) candidates \
         WHERE day < $1 \
           AND NOT EXISTS ( \
               SELECT 1 FROM usage_rollup_days r \
               WHERE r.day = candidates.day AND r.finalized \
           ) \
         ORDER BY day ASC LIMIT $3",
    )
    .bind(today)
    .bind(today_start)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(days)
}

// ---------------------------------------------------------------------------
// 保留清理（S5.1）：先日结、后清理；明细仅删已 finalized 且过期的日
// ---------------------------------------------------------------------------

/// 删除一批过期明细（仅 finalized 日；`ctid` 子查询避免长锁）。
pub async fn delete_expired_detail_batch(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    batch_size: i64,
) -> Result<u64, AppError> {
    let result = sqlx::query(
        "WITH batch AS ( \
             SELECT ctid FROM usage_records \
             WHERE created_at < $1 \
               AND (created_at AT TIME ZONE 'Asia/Shanghai')::date IN ( \
                   SELECT day FROM usage_rollup_days WHERE finalized = true \
               ) \
             LIMIT $2 \
         ) \
         DELETE FROM usage_records u USING batch WHERE u.ctid = batch.ctid",
    )
    .bind(cutoff)
    .bind(batch_size)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// 删除过期的 rollup 聚合与日结元数据（按 rollup 保留期）。
pub async fn delete_expired_rollups(
    pool: &PgPool,
    cutoff_day: NaiveDate,
) -> Result<(u64, u64), AppError> {
    let rollups = sqlx::query("DELETE FROM daily_usage_rollups WHERE day < $1")
        .bind(cutoff_day)
        .execute(pool)
        .await?
        .rows_affected();
    let days = sqlx::query("DELETE FROM usage_rollup_days WHERE day < $1")
        .bind(cutoff_day)
        .execute(pool)
        .await?
        .rows_affected();
    Ok((rollups, days))
}

// ---------------------------------------------------------------------------
// 读路径（S5/S5.1）：明细聚合 + 已结账日 rollup 聚合；事务由 service 持有
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AggregateFilters {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub user_id: Option<Uuid>,
    pub key_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub model_alias: Option<String>,
    /// 从明细聚合中排除的上海日（这些整日已 finalized，由 rollup 提供）。
    pub exclude_days: Vec<NaiveDate>,
}

#[derive(Debug, Clone, Default)]
pub struct AggregateMetrics {
    pub requests: i64,
    pub success_requests: i64,
    pub error_requests: i64,
    pub rejected_requests: i64,
    pub client_disconnect_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: Decimal,
}

#[derive(Debug, Clone)]
pub struct GroupAggregate {
    pub day: Option<NaiveDate>,
    pub model_alias: Option<String>,
    pub key_id: Option<Uuid>,
    pub metrics: AggregateMetrics,
}

const METRIC_SELECT: &str = "count(*) AS requests, \
    count(*) FILTER (WHERE status = 'success') AS success_requests, \
    count(*) FILTER (WHERE status IN ('upstream_error', 'timeout', 'gateway_error')) AS error_requests, \
    count(*) FILTER (WHERE status = 'rejected') AS rejected_requests, \
    count(*) FILTER (WHERE status = 'client_disconnect') AS client_disconnect_requests, \
    coalesce(sum(input_tokens), 0)::bigint AS input_tokens, \
    coalesce(sum(output_tokens), 0)::bigint AS output_tokens, \
    coalesce(sum(cost), 0)::text AS cost";

const FILTER_SQL: &str = "created_at >= $1 AND created_at < $2 \
    AND ($3::uuid IS NULL OR user_id = $3) \
    AND ($4::uuid IS NULL OR key_id = $4) \
    AND ($5::uuid IS NULL OR provider_id = $5) \
    AND ($6::text IS NULL OR model_alias = $6) \
    AND NOT ((created_at AT TIME ZONE 'Asia/Shanghai')::date = ANY($7::date[]))";

const ROLLUP_FILTER_SQL: &str = "day = ANY($1::date[]) \
    AND ($2::uuid IS NULL OR user_id = $2) \
    AND ($3::uuid IS NULL OR key_id = $3) \
    AND ($4::uuid IS NULL OR provider_id = $4) \
    AND ($5::text IS NULL OR model_alias = $5)";

const ROLLUP_METRIC_SELECT: &str = "coalesce(sum(requests), 0)::bigint AS requests, \
    coalesce(sum(success_requests), 0)::bigint AS success_requests, \
    coalesce(sum(error_requests), 0)::bigint AS error_requests, \
    coalesce(sum(rejected_requests), 0)::bigint AS rejected_requests, \
    coalesce(sum(client_disconnect_requests), 0)::bigint AS client_disconnect_requests, \
    coalesce(sum(input_tokens), 0)::bigint AS input_tokens, \
    coalesce(sum(output_tokens), 0)::bigint AS output_tokens, \
    coalesce(sum(cost), 0)::text AS cost";

fn bind_aggregate_filters<'q>(
    query: Query<'q, sqlx::Postgres, PgArguments>,
    filters: &'q AggregateFilters,
) -> Query<'q, sqlx::Postgres, PgArguments> {
    query
        .bind(filters.start)
        .bind(filters.end)
        .bind(filters.user_id)
        .bind(filters.key_id)
        .bind(filters.provider_id)
        .bind(filters.model_alias.as_deref())
        .bind(&filters.exclude_days)
}

fn bind_aggregate_filters_scalar<'q, O>(
    query: QueryScalar<'q, sqlx::Postgres, O, PgArguments>,
    filters: &'q AggregateFilters,
) -> QueryScalar<'q, sqlx::Postgres, O, PgArguments> {
    query
        .bind(filters.start)
        .bind(filters.end)
        .bind(filters.user_id)
        .bind(filters.key_id)
        .bind(filters.provider_id)
        .bind(filters.model_alias.as_deref())
        .bind(&filters.exclude_days)
}

fn group_expression(dim: &str) -> &'static str {
    match dim {
        "day" => "(created_at AT TIME ZONE 'Asia/Shanghai')::date AS day",
        "model_alias" => "model_alias",
        "key_id" => "key_id",
        _ => panic!("group dimension validated by service"),
    }
}

fn group_plain(dim: &str) -> &'static str {
    match dim {
        "day" => "(created_at AT TIME ZONE 'Asia/Shanghai')::date",
        "model_alias" => "model_alias",
        "key_id" => "key_id",
        _ => panic!("group dimension validated by service"),
    }
}

fn rollup_group_column(dim: &str) -> &'static str {
    match dim {
        "day" => "day",
        "model_alias" => "model_alias",
        "key_id" => "key_id",
        _ => panic!("group dimension validated by service"),
    }
}

fn extract_metrics(row: &PgRow) -> AggregateMetrics {
    AggregateMetrics {
        requests: row.try_get("requests").unwrap_or(0),
        success_requests: row.try_get("success_requests").unwrap_or(0),
        error_requests: row.try_get("error_requests").unwrap_or(0),
        rejected_requests: row.try_get("rejected_requests").unwrap_or(0),
        client_disconnect_requests: row.try_get("client_disconnect_requests").unwrap_or(0),
        input_tokens: row.try_get("input_tokens").unwrap_or(0),
        output_tokens: row.try_get("output_tokens").unwrap_or(0),
        cost: row
            .try_get::<String, _>("cost")
            .ok()
            .and_then(|raw| Decimal::from_str(&raw).ok())
            .unwrap_or(Decimal::ZERO),
    }
}

fn extract_group(row: &PgRow) -> GroupAggregate {
    GroupAggregate {
        day: row.try_get::<Option<NaiveDate>, _>("day").ok().flatten(),
        model_alias: row
            .try_get::<Option<String>, _>("model_alias")
            .ok()
            .flatten(),
        key_id: row.try_get::<Option<Uuid>, _>("key_id").ok().flatten(),
        metrics: extract_metrics(row),
    }
}

/// 已 finalized 的上海日集合（升序）：`[start_day, end_day)`。
pub async fn finalized_days(
    conn: &mut PgConnection,
    start_day: NaiveDate,
    end_day: NaiveDate,
) -> Result<Vec<NaiveDate>, AppError> {
    let days = sqlx::query_scalar::<_, NaiveDate>(
        "SELECT day FROM usage_rollup_days \
         WHERE finalized = true AND day >= $1 AND day < $2 ORDER BY day",
    )
    .bind(start_day)
    .bind(end_day)
    .fetch_all(conn)
    .await?;
    Ok(days)
}

pub async fn aggregate_totals(
    conn: &mut PgConnection,
    filters: &AggregateFilters,
) -> Result<AggregateMetrics, AppError> {
    let sql = format!("SELECT {METRIC_SELECT} FROM usage_records WHERE {FILTER_SQL}");
    let row = bind_aggregate_filters(sqlx::query(&sql), filters)
        .fetch_one(conn)
        .await?;
    Ok(extract_metrics(&row))
}

pub async fn aggregate_grouped(
    conn: &mut PgConnection,
    filters: &AggregateFilters,
    group_by: &[&str],
    limit: i64,
    offset: i64,
) -> Result<(Vec<GroupAggregate>, i64), AppError> {
    let (select, group, order) = grouped_parts(group_by);
    let sql = format!(
        "SELECT {select} FROM usage_records WHERE {FILTER_SQL} \
         GROUP BY {group} ORDER BY {order} ASC LIMIT $8 OFFSET $9"
    );
    let rows = bind_aggregate_filters(sqlx::query(&sql), filters)
        .bind(limit)
        .bind(offset)
        .fetch_all(&mut *conn)
        .await?;
    let items = rows.iter().map(extract_group).collect();

    let count_sql = format!(
        "SELECT count(*) FROM (SELECT 1 FROM usage_records WHERE {FILTER_SQL} \
         GROUP BY {group}) counted"
    );
    let total: i64 = bind_aggregate_filters_scalar(sqlx::query_scalar(&count_sql), filters)
        .fetch_one(&mut *conn)
        .await?;
    Ok((items, total))
}

/// 不分页的分组聚合（S5.1 合并 finalized rollup 与明细后内存分页）。
pub async fn aggregate_grouped_all(
    conn: &mut PgConnection,
    filters: &AggregateFilters,
    group_by: &[&str],
) -> Result<Vec<GroupAggregate>, AppError> {
    let (select, group, _) = grouped_parts(group_by);
    let sql = format!("SELECT {select} FROM usage_records WHERE {FILTER_SQL} GROUP BY {group}");
    let rows = bind_aggregate_filters(sqlx::query(&sql), filters)
        .fetch_all(conn)
        .await?;
    Ok(rows.iter().map(extract_group).collect())
}

/// 已结账日按日聚合（overview trend/totals；只含 finalized 日）。
pub async fn rollup_aggregate_days(
    conn: &mut PgConnection,
    days: &[NaiveDate],
    filters: &AggregateFilters,
) -> Result<Vec<(NaiveDate, AggregateMetrics)>, AppError> {
    if days.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT day, {ROLLUP_METRIC_SELECT} FROM daily_usage_rollups \
         WHERE {ROLLUP_FILTER_SQL} GROUP BY day ORDER BY day"
    );
    let rows = sqlx::query(&sql)
        .bind(days)
        .bind(filters.user_id)
        .bind(filters.key_id)
        .bind(filters.provider_id)
        .bind(filters.model_alias.as_deref())
        .fetch_all(conn)
        .await?;
    rows.iter()
        .map(|row| {
            let day = row.try_get::<NaiveDate, _>("day").map_err(AppError::from)?;
            Ok((day, extract_metrics(row)))
        })
        .collect()
}

/// 已结账日分组聚合（usage 分组；无分页，service 合并后内存分页）。
pub async fn rollup_aggregate_grouped(
    conn: &mut PgConnection,
    days: &[NaiveDate],
    filters: &AggregateFilters,
    group_by: &[&str],
) -> Result<Vec<GroupAggregate>, AppError> {
    if days.is_empty() {
        return Ok(Vec::new());
    }
    let dims = group_by
        .iter()
        .map(|dim| rollup_group_column(dim))
        .collect::<Vec<_>>();
    let select = format!("{}, {ROLLUP_METRIC_SELECT}", dims.join(", "));
    let sql = format!(
        "SELECT {select} FROM daily_usage_rollups WHERE {ROLLUP_FILTER_SQL} GROUP BY {}",
        dims.join(", ")
    );
    let rows = sqlx::query(&sql)
        .bind(days)
        .bind(filters.user_id)
        .bind(filters.key_id)
        .bind(filters.provider_id)
        .bind(filters.model_alias.as_deref())
        .fetch_all(conn)
        .await?;
    Ok(rows.iter().map(extract_group).collect())
}

fn grouped_parts(group_by: &[&str]) -> (String, String, String) {
    let select_dims = group_by
        .iter()
        .map(|dim| group_expression(dim))
        .collect::<Vec<_>>();
    let group_cols = group_by
        .iter()
        .map(|dim| group_plain(dim))
        .collect::<Vec<_>>();
    let select = format!("{}, {METRIC_SELECT}", select_dims.join(", "));
    let group = group_cols.join(", ");
    let order = (1..=group_by.len())
        .map(|idx| idx.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    (select, group, order)
}

/// 当日状态分布（overview.today.status_breakdown）。
pub async fn status_breakdown(
    conn: &mut PgConnection,
    filters: &AggregateFilters,
) -> Result<Vec<(String, i64)>, AppError> {
    let sql = format!(
        "SELECT status, count(*)::bigint AS count FROM usage_records \
         WHERE {FILTER_SQL} GROUP BY status ORDER BY status"
    );
    let rows = bind_aggregate_filters(sqlx::query(&sql), filters)
        .fetch_all(conn)
        .await?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.try_get::<String, _>("status").unwrap_or_default(),
                row.try_get::<i64, _>("count").unwrap_or(0),
            )
        })
        .collect())
}

// ---------------------------------------------------------------------------
// 单条读取（observability 请求详情合并）
// ---------------------------------------------------------------------------

/// 单条用量读取（observability 请求详情合并；不存在返回 None）。
pub async fn get_usage_by_request_id(
    pool: &PgPool,
    request_id: &str,
) -> Result<Option<UsageBlockRow>, AppError> {
    let row = sqlx::query(
        "SELECT input_tokens, output_tokens, cost::text AS cost, currency, latency_ms, \
         ttft_ms, is_stream, fallback, status, http_status, error_code, usage_source \
         FROM usage_records WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(UsageBlockRow {
        input_tokens: row.try_get("input_tokens").unwrap_or(0),
        output_tokens: row.try_get("output_tokens").unwrap_or(0),
        cost: row.try_get("cost").unwrap_or_else(|_| "0".to_string()),
        currency: row
            .try_get("currency")
            .unwrap_or_else(|_| "CNY".to_string()),
        latency_ms: row.try_get("latency_ms").unwrap_or(0),
        ttft_ms: row.try_get("ttft_ms").ok().flatten(),
        is_stream: row.try_get("is_stream").unwrap_or(false),
        fallback: row.try_get("fallback").unwrap_or(false),
        status: row.try_get("status").unwrap_or_default(),
        http_status: row.try_get("http_status").ok().flatten(),
        error_code: row.try_get("error_code").ok().flatten(),
        usage_source: row.try_get("usage_source").unwrap_or_default(),
    }))
}

#[derive(Debug, Clone)]
pub struct UsageBlockRow {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cost: String,
    pub currency: String,
    pub latency_ms: i32,
    pub ttft_ms: Option<i32>,
    pub is_stream: bool,
    pub fallback: bool,
    pub status: String,
    pub http_status: Option<i32>,
    pub error_code: Option<String>,
    pub usage_source: String,
}
