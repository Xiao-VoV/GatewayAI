//! catalog 数据访问（表归属：`model_groups` / `deployments` / `catalog_state`）。
//!
//! 只读写 catalog 自有表；providers/access 的只读数据经 service 获取。
//! 所有写路径由 service 在同一事务内调用 `bump_version`。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::platform::error::AppError;

use super::model::{DeploymentSpec, GroupFields};

const GROUP_COLUMNS: &str = "id, client_model_name, display_name, canonical, description, \
     capabilities, context_window, max_output, protocols, parameters, \
     price_input::float8 AS price_input, price_output::float8 AS price_output, currency, status, \
     created_at, updated_at";

const DEPLOYMENT_COLUMNS: &str = "id, model_group_id, upstream_model_id, provider_account_id, \
     priority, weight, enabled, notes, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ModelGroupRow {
    pub id: Uuid,
    pub client_model_name: String,
    pub display_name: String,
    pub canonical: String,
    pub description: String,
    pub capabilities: serde_json::Value,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub protocols: serde_json::Value,
    pub parameters: serde_json::Value,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeploymentRow {
    pub id: Uuid,
    pub model_group_id: Option<Uuid>,
    pub upstream_model_id: Uuid,
    pub provider_account_id: Uuid,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub notes: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 部署列表过滤条件。
pub struct DeploymentFilters {
    pub model_group_id: Option<Uuid>,
    pub upstream_model_id: Option<Uuid>,
    /// provider_id 过滤：由 service 经 providers 只读视图解析为上游模型 ID 集合。
    pub provider_upstream_ids: Option<Vec<Uuid>>,
    pub provider_account_id: Option<Uuid>,
    pub enabled: Option<bool>,
    pub ungrouped: Option<bool>,
    pub limit: i64,
    pub offset: i64,
}

fn decimal_bind(value: Option<f64>) -> Option<String> {
    value.map(|value| format!("{value:.6}"))
}

// ---------- catalog_state 版本 ----------

/// 目录版本 +1（必须在业务写同一事务内调用）。
pub async fn bump_version(conn: &mut PgConnection) -> Result<(), AppError> {
    let affected = sqlx::query(
        "UPDATE catalog_state SET version = version + 1, updated_at = now() WHERE id = 1",
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if affected != 1 {
        return Err(AppError::internal(
            "catalog_state singleton row is missing; run migration 0009",
        ));
    }
    Ok(())
}

// ---------- 模型组 ----------

pub async fn list_model_groups(
    pool: &PgPool,
    status: Option<&str>,
    canonical: Option<&str>,
    q: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ModelGroupRow>, i64), AppError> {
    let rows = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "SELECT {GROUP_COLUMNS} FROM model_groups \
         WHERE ($1::text IS NULL OR status = $1) \
           AND ($2::text IS NULL OR canonical = $2) \
           AND ($3::text IS NULL OR client_model_name ILIKE '%' || $3 || '%' \
                OR display_name ILIKE '%' || $3 || '%') \
         ORDER BY created_at DESC, id DESC LIMIT $4 OFFSET $5"
    ))
    .bind(status)
    .bind(canonical)
    .bind(q)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM model_groups \
         WHERE ($1::text IS NULL OR status = $1) \
           AND ($2::text IS NULL OR canonical = $2) \
           AND ($3::text IS NULL OR client_model_name ILIKE '%' || $3 || '%' \
                OR display_name ILIKE '%' || $3 || '%')",
    )
    .bind(status)
    .bind(canonical)
    .bind(q)
    .fetch_one(pool)
    .await?;
    Ok((rows, total))
}

pub async fn get_model_group(pool: &PgPool, id: Uuid) -> Result<Option<ModelGroupRow>, AppError> {
    let row = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "SELECT {GROUP_COLUMNS} FROM model_groups WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_model_group_for_update(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<ModelGroupRow>, AppError> {
    let row = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "SELECT {GROUP_COLUMNS} FROM model_groups WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

pub async fn get_model_group_conn(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<ModelGroupRow>, AppError> {
    let row = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "SELECT {GROUP_COLUMNS} FROM model_groups WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

pub async fn list_group_client_names(
    pool: &PgPool,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, String>, AppError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, client_model_name FROM model_groups WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

pub async fn insert_model_group(
    conn: &mut PgConnection,
    id: Uuid,
    fields: &GroupFields,
) -> Result<ModelGroupRow, AppError> {
    let capabilities = serde_json::json!(fields.capabilities);
    let protocols = serde_json::json!(fields.protocols);
    let result = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "INSERT INTO model_groups \
         (id, client_model_name, display_name, canonical, description, capabilities, \
          context_window, max_output, protocols, parameters, price_input, price_output, \
          currency, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
                 $11::text::numeric, $12::text::numeric, $13, $14) \
         RETURNING {GROUP_COLUMNS}"
    ))
    .bind(id)
    .bind(&fields.client_model_name)
    .bind(&fields.display_name)
    .bind(&fields.canonical)
    .bind(&fields.description)
    .bind(capabilities)
    .bind(fields.context_window)
    .bind(fields.max_output)
    .bind(protocols)
    .bind(&fields.parameters)
    .bind(decimal_bind(fields.price_input))
    .bind(decimal_bind(fields.price_output))
    .bind(&fields.currency)
    .bind(&fields.status)
    .fetch_one(&mut *conn)
    .await;
    result.map_err(map_group_write_error)
}

pub async fn update_model_group(
    conn: &mut PgConnection,
    id: Uuid,
    fields: &GroupFields,
) -> Result<Option<ModelGroupRow>, AppError> {
    let capabilities = serde_json::json!(fields.capabilities);
    let protocols = serde_json::json!(fields.protocols);
    let result = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "UPDATE model_groups SET \
         client_model_name = $2, display_name = $3, canonical = $4, description = $5, \
         capabilities = $6, context_window = $7, max_output = $8, protocols = $9, \
         parameters = $10, price_input = $11::text::numeric, price_output = $12::text::numeric, \
         currency = $13, status = $14, updated_at = now() \
         WHERE id = $1 RETURNING {GROUP_COLUMNS}"
    ))
    .bind(id)
    .bind(&fields.client_model_name)
    .bind(&fields.display_name)
    .bind(&fields.canonical)
    .bind(&fields.description)
    .bind(capabilities)
    .bind(fields.context_window)
    .bind(fields.max_output)
    .bind(protocols)
    .bind(&fields.parameters)
    .bind(decimal_bind(fields.price_input))
    .bind(decimal_bind(fields.price_output))
    .bind(&fields.currency)
    .bind(&fields.status)
    .fetch_optional(&mut *conn)
    .await;
    result.map_err(map_group_write_error)
}

/// 归档（软删除）：仅当当前非 archived 时更新；返回是否发生状态变化。
pub async fn archive_model_group(conn: &mut PgConnection, id: Uuid) -> Result<bool, AppError> {
    let affected = sqlx::query(
        "UPDATE model_groups SET status = 'archived', updated_at = now() \
         WHERE id = $1 AND status <> 'archived'",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn touch_model_group(conn: &mut PgConnection, id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE model_groups SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// 组内部署计数（总数，启用数）。
pub async fn count_group_deployments(
    conn: &mut PgConnection,
    group_id: Uuid,
) -> Result<(i64, i64), AppError> {
    let row = sqlx::query_as::<_, (i64, i64)>(
        "SELECT count(*), count(*) FILTER (WHERE enabled) FROM deployments \
         WHERE model_group_id = $1",
    )
    .bind(group_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// 启用部署数量（发布就绪校验）。
pub async fn count_enabled_deployments(pool: &PgPool, group_id: Uuid) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM deployments WHERE model_group_id = $1 AND enabled",
    )
    .bind(group_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// 组内部署引用的上游模型 ID（canonical 校验用）。
pub async fn list_group_upstream_ids(
    conn: &mut PgConnection,
    group_id: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    let rows = sqlx::query_scalar::<_, Uuid>(
        "SELECT upstream_model_id FROM deployments WHERE model_group_id = $1",
    )
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

// ---------- 部署 ----------

pub async fn list_deployments_for_groups(
    pool: &PgPool,
    group_ids: &[Uuid],
) -> Result<Vec<DeploymentRow>, AppError> {
    if group_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, DeploymentRow>(&format!(
        "SELECT {DEPLOYMENT_COLUMNS} FROM deployments \
         WHERE model_group_id = ANY($1) ORDER BY model_group_id, priority, id"
    ))
    .bind(group_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_deployments_for_group(
    pool: &PgPool,
    group_id: Uuid,
) -> Result<Vec<DeploymentRow>, AppError> {
    let rows = sqlx::query_as::<_, DeploymentRow>(&format!(
        "SELECT {DEPLOYMENT_COLUMNS} FROM deployments WHERE model_group_id = $1 \
         ORDER BY priority, id"
    ))
    .bind(group_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_deployments(
    pool: &PgPool,
    filters: &DeploymentFilters,
) -> Result<(Vec<DeploymentRow>, i64), AppError> {
    let where_clause = "WHERE ($1::uuid IS NULL OR model_group_id = $1) \
         AND ($2::uuid IS NULL OR upstream_model_id = $2) \
         AND ($3::uuid[] IS NULL OR upstream_model_id = ANY($3)) \
         AND ($4::uuid IS NULL OR provider_account_id = $4) \
         AND ($5::boolean IS NULL OR enabled = $5) \
         AND ($6::boolean IS NULL OR (model_group_id IS NULL) = $6)";
    let rows = sqlx::query_as::<_, DeploymentRow>(&format!(
        "SELECT {DEPLOYMENT_COLUMNS} FROM deployments {where_clause} \
         ORDER BY created_at DESC, id DESC LIMIT $7 OFFSET $8"
    ))
    .bind(filters.model_group_id)
    .bind(filters.upstream_model_id)
    .bind(filters.provider_upstream_ids.clone())
    .bind(filters.provider_account_id)
    .bind(filters.enabled)
    .bind(filters.ungrouped)
    .bind(filters.limit)
    .bind(filters.offset)
    .fetch_all(pool)
    .await?;
    let total =
        sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM deployments {where_clause}"))
            .bind(filters.model_group_id)
            .bind(filters.upstream_model_id)
            .bind(filters.provider_upstream_ids.clone())
            .bind(filters.provider_account_id)
            .bind(filters.enabled)
            .bind(filters.ungrouped)
            .fetch_one(pool)
            .await?;
    Ok((rows, total))
}

pub async fn get_deployment(pool: &PgPool, id: Uuid) -> Result<Option<DeploymentRow>, AppError> {
    let row = sqlx::query_as::<_, DeploymentRow>(&format!(
        "SELECT {DEPLOYMENT_COLUMNS} FROM deployments WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_deployment_for_update(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<DeploymentRow>, AppError> {
    let row = sqlx::query_as::<_, DeploymentRow>(&format!(
        "SELECT {DEPLOYMENT_COLUMNS} FROM deployments WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

pub async fn insert_deployment(
    conn: &mut PgConnection,
    id: Uuid,
    spec: &DeploymentSpec,
    model_group_id: Option<Uuid>,
) -> Result<DeploymentRow, AppError> {
    let result = sqlx::query_as::<_, DeploymentRow>(&format!(
        "INSERT INTO deployments \
         (id, model_group_id, upstream_model_id, provider_account_id, priority, weight, enabled, notes) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {DEPLOYMENT_COLUMNS}"
    ))
    .bind(id)
    .bind(model_group_id)
    .bind(spec.upstream_model_id)
    .bind(spec.provider_account_id)
    .bind(spec.priority)
    .bind(spec.weight)
    .bind(spec.enabled)
    .bind(&spec.notes)
    .fetch_one(&mut *conn)
    .await;
    result.map_err(map_deployment_write_error)
}

pub async fn update_deployment(
    conn: &mut PgConnection,
    id: Uuid,
    spec: &DeploymentSpec,
    model_group_id: Option<Uuid>,
) -> Result<Option<DeploymentRow>, AppError> {
    let result = sqlx::query_as::<_, DeploymentRow>(&format!(
        "UPDATE deployments SET model_group_id = $2, upstream_model_id = $3, \
         provider_account_id = $4, priority = $5, weight = $6, enabled = $7, notes = $8, \
         updated_at = now() WHERE id = $1 RETURNING {DEPLOYMENT_COLUMNS}"
    ))
    .bind(id)
    .bind(model_group_id)
    .bind(spec.upstream_model_id)
    .bind(spec.provider_account_id)
    .bind(spec.priority)
    .bind(spec.weight)
    .bind(spec.enabled)
    .bind(&spec.notes)
    .fetch_optional(&mut *conn)
    .await;
    result.map_err(map_deployment_write_error)
}

pub async fn delete_deployment(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<DeploymentRow>, AppError> {
    let row = sqlx::query_as::<_, DeploymentRow>(&format!(
        "DELETE FROM deployments WHERE id = $1 RETURNING {DEPLOYMENT_COLUMNS}"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

// ---------- 错误映射 ----------

fn constraint_name(error: &sqlx::Error) -> Option<&str> {
    match error {
        sqlx::Error::Database(database_error) => database_error.constraint(),
        _ => None,
    }
}

fn map_group_write_error(error: sqlx::Error) -> AppError {
    match constraint_name(&error) {
        Some("model_groups_client_model_name_key") => {
            AppError::conflict("client_model_name_conflict", "客户端模型名已被占用", None)
        }
        Some(_) | None => AppError::from(error),
    }
}

fn map_deployment_write_error(error: sqlx::Error) -> AppError {
    match constraint_name(&error) {
        Some("deployments_upstream_account_key") => {
            AppError::conflict("deployment_conflict", "该上游模型在该账号下已有部署", None)
        }
        Some("deployments_upstream_model_id_fkey") => {
            AppError::not_found("upstream_model_not_found", "上游模型不存在")
        }
        Some("deployments_provider_account_id_fkey") => {
            AppError::not_found("provider_account_not_found", "渠道账号不存在")
        }
        Some("deployments_model_group_id_fkey") => {
            AppError::not_found("model_group_not_found", "模型组不存在")
        }
        Some(_) | None => AppError::from(error),
    }
}

// ===========================================================================
// 发布（Publication）与客户端投影读取（迭代 2）
// ===========================================================================

const PUBLICATION_COLUMNS: &str = "p.id, p.model_group_id, \
     g.client_model_name AS model_group_client_name, p.status, p.data_level_max, p.quota_rpm, \
     p.quota_tpm, p.quota_daily_requests, p.note, p.created_by, p.created_at, p.updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PublicationRow {
    pub id: Uuid,
    pub model_group_id: Uuid,
    pub model_group_client_name: String,
    pub status: String,
    pub data_level_max: Option<i16>,
    pub quota_rpm: Option<i32>,
    pub quota_tpm: Option<i64>,
    pub quota_daily_requests: Option<i64>,
    pub note: String,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PublicationTargetRow {
    pub id: Uuid,
    pub publication_id: Uuid,
    pub subject_type: String,
    pub subject_id: Option<Uuid>,
    pub visible: bool,
    pub allowed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ActivePublicationRow {
    pub id: Uuid,
    pub model_group_id: Uuid,
}

/// 客户端发布投影所需的、版本一致的只读快照行集。
pub struct ProjectionRows {
    pub version: i64,
    pub groups: Vec<ModelGroupRow>,
    pub deployments: Vec<DeploymentRow>,
    pub publications: Vec<ActivePublicationRow>,
    pub targets: Vec<PublicationTargetRow>,
    pub tool_defaults: Vec<ProjectionToolDefaultRow>,
    pub tool_default_models: Vec<ProjectionToolDefaultModelRow>,
}

/// 在 `REPEATABLE READ READ ONLY` 事务内读取 `catalog_state.version` + 组/部署/发布/targets。
///
/// 先读 version 再读数据：同一快照内保证 `catalog_version` 与实际配置一致。
pub async fn read_projection_state(pool: &PgPool) -> Result<ProjectionRows, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let version = sqlx::query_scalar::<_, i64>("SELECT version FROM catalog_state WHERE id = 1")
        .fetch_one(&mut *tx)
        .await?;
    let groups = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "SELECT {GROUP_COLUMNS} FROM model_groups WHERE status = 'active' \
         ORDER BY client_model_name, id"
    ))
    .fetch_all(&mut *tx)
    .await?;
    let group_ids: Vec<Uuid> = groups.iter().map(|group| group.id).collect();
    let deployments = if group_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, DeploymentRow>(&format!(
            "SELECT {DEPLOYMENT_COLUMNS} FROM deployments WHERE model_group_id = ANY($1) \
             ORDER BY model_group_id, priority, id"
        ))
        .bind(&group_ids)
        .fetch_all(&mut *tx)
        .await?
    };
    let publications = if group_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, ActivePublicationRow>(
            "SELECT id, model_group_id FROM publications \
             WHERE status = 'active' AND model_group_id = ANY($1) ORDER BY id",
        )
        .bind(&group_ids)
        .fetch_all(&mut *tx)
        .await?
    };
    let publication_ids: Vec<Uuid> = publications
        .iter()
        .map(|publication| publication.id)
        .collect();
    let targets = if publication_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, PublicationTargetRow>(
            "SELECT id, publication_id, subject_type, subject_id, visible, allowed \
             FROM publication_targets WHERE publication_id = ANY($1) ORDER BY id",
        )
        .bind(&publication_ids)
        .fetch_all(&mut *tx)
        .await?
    };
    let tool_defaults = sqlx::query_as::<_, ProjectionToolDefaultRow>(
        "SELECT id, tool_key, scope_type, scope_id, default_model_group_id, allow_switch, \
         updated_at FROM tool_model_defaults ORDER BY tool_key, scope_type, id",
    )
    .fetch_all(&mut *tx)
    .await?;
    let tool_default_ids: Vec<Uuid> = tool_defaults.iter().map(|default| default.id).collect();
    let tool_default_models = if tool_default_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, ProjectionToolDefaultModelRow>(
            "SELECT tool_default_id, model_group_id FROM tool_model_default_models \
             WHERE tool_default_id = ANY($1) ORDER BY tool_default_id, model_group_id",
        )
        .bind(&tool_default_ids)
        .fetch_all(&mut *tx)
        .await?
    };
    tx.commit().await?;
    Ok(ProjectionRows {
        version,
        groups,
        deployments,
        publications,
        targets,
        tool_defaults,
        tool_default_models,
    })
}

/// routing 解析所需的、版本一致的只读快照行集（包含 draft/archived 组）。
pub struct RoutingRows {
    pub groups: Vec<ModelGroupRow>,
    pub deployments: Vec<DeploymentRow>,
}

/// 在 `REPEATABLE READ READ ONLY` 事务内读取 `catalog_state.version` + 全部组/部署。
///
/// 与 [`read_projection_state`] 不同：这里返回**全部状态**的组（routing 自己决定
/// active 优先、draft/archived 不遮蔽 alias），且不读发布数据（授权由 proxy 调
/// `authorize_model`）。
pub async fn read_routing_state(pool: &PgPool) -> Result<RoutingRows, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let groups = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "SELECT {GROUP_COLUMNS} FROM model_groups ORDER BY client_model_name, id"
    ))
    .fetch_all(&mut *tx)
    .await?;
    let group_ids: Vec<Uuid> = groups.iter().map(|group| group.id).collect();
    let deployments = if group_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, DeploymentRow>(&format!(
            "SELECT {DEPLOYMENT_COLUMNS} FROM deployments WHERE model_group_id = ANY($1)              ORDER BY model_group_id, priority, id"
        ))
        .bind(&group_ids)
        .fetch_all(&mut *tx)
        .await?
    };
    tx.commit().await?;
    Ok(RoutingRows {
        groups,
        deployments,
    })
}

/// 发布列表过滤条件。
pub struct PublicationFilters {
    pub model_group_id: Option<Uuid>,
    pub status: Option<String>,
    pub subject_type: Option<String>,
    pub subject_id: Option<Uuid>,
    pub limit: i64,
    pub offset: i64,
}

fn publication_where_clause() -> &'static str {
    "WHERE ($1::uuid IS NULL OR p.model_group_id = $1) \
       AND ($2::text IS NULL OR p.status = $2) \
       AND ($3::text IS NULL OR EXISTS (SELECT 1 FROM publication_targets t \
            WHERE t.publication_id = p.id AND t.subject_type = $3)) \
       AND ($4::uuid IS NULL OR EXISTS (SELECT 1 FROM publication_targets t \
            WHERE t.publication_id = p.id AND t.subject_id = $4))"
}

pub async fn list_publications(
    pool: &PgPool,
    filters: &PublicationFilters,
) -> Result<(Vec<PublicationRow>, i64), AppError> {
    let where_clause = publication_where_clause();
    let rows = sqlx::query_as::<_, PublicationRow>(&format!(
        "SELECT {PUBLICATION_COLUMNS} FROM publications p \
         JOIN model_groups g ON g.id = p.model_group_id {where_clause} \
         ORDER BY p.created_at DESC, p.id DESC LIMIT $5 OFFSET $6"
    ))
    .bind(filters.model_group_id)
    .bind(filters.status.as_deref())
    .bind(filters.subject_type.as_deref())
    .bind(filters.subject_id)
    .bind(filters.limit)
    .bind(filters.offset)
    .fetch_all(pool)
    .await?;
    let total = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT count(*) FROM publications p {where_clause}"
    ))
    .bind(filters.model_group_id)
    .bind(filters.status.as_deref())
    .bind(filters.subject_type.as_deref())
    .bind(filters.subject_id)
    .fetch_one(pool)
    .await?;
    Ok((rows, total))
}

pub async fn get_publication(pool: &PgPool, id: Uuid) -> Result<Option<PublicationRow>, AppError> {
    let row = sqlx::query_as::<_, PublicationRow>(&format!(
        "SELECT {PUBLICATION_COLUMNS} FROM publications p \
         JOIN model_groups g ON g.id = p.model_group_id WHERE p.id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_publication_for_update(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<PublicationRow>, AppError> {
    let row = sqlx::query_as::<_, PublicationRow>(&format!(
        "SELECT {PUBLICATION_COLUMNS} FROM publications p \
         JOIN model_groups g ON g.id = p.model_group_id WHERE p.id = $1 FOR UPDATE OF p"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

pub async fn list_targets_for_publications(
    pool: &PgPool,
    publication_ids: &[Uuid],
) -> Result<Vec<PublicationTargetRow>, AppError> {
    if publication_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, PublicationTargetRow>(
        "SELECT id, publication_id, subject_type, subject_id, visible, allowed \
         FROM publication_targets WHERE publication_id = ANY($1) ORDER BY publication_id, id",
    )
    .bind(publication_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn insert_publication(
    conn: &mut PgConnection,
    id: Uuid,
    fields: &super::model::PublicationFields,
    created_by: Option<Uuid>,
) -> Result<(), AppError> {
    let result = sqlx::query(
        "INSERT INTO publications \
         (id, model_group_id, status, data_level_max, quota_rpm, quota_tpm, \
          quota_daily_requests, note, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(fields.model_group_id)
    .bind(&fields.status)
    .bind(fields.data_level_max)
    .bind(fields.quota_rpm)
    .bind(fields.quota_tpm)
    .bind(fields.quota_daily_requests)
    .bind(&fields.note)
    .bind(created_by)
    .execute(&mut *conn)
    .await;
    result.map_err(map_publication_write_error)?;
    insert_publication_targets(conn, id, &fields.targets).await?;
    Ok(())
}

async fn insert_publication_targets(
    conn: &mut PgConnection,
    publication_id: Uuid,
    targets: &[super::model::PublicationTargetSpec],
) -> Result<(), AppError> {
    for target in targets {
        sqlx::query(
            "INSERT INTO publication_targets \
             (id, publication_id, subject_type, subject_id, visible, allowed) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(publication_id)
        .bind(&target.subject_type)
        .bind(target.subject_id)
        .bind(target.visible)
        .bind(target.allowed)
        .execute(&mut *conn)
        .await
        .map_err(map_publication_write_error)?;
    }
    Ok(())
}

/// 全量替换发布对象（事务内先删后插；不在本函数内 bump 版本）。
pub async fn replace_publication_targets(
    conn: &mut PgConnection,
    publication_id: Uuid,
    targets: &[super::model::PublicationTargetSpec],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM publication_targets WHERE publication_id = $1")
        .bind(publication_id)
        .execute(&mut *conn)
        .await?;
    insert_publication_targets(conn, publication_id, targets).await
}

pub async fn update_publication(
    conn: &mut PgConnection,
    id: Uuid,
    fields: &super::model::PublicationFields,
) -> Result<bool, AppError> {
    let affected = sqlx::query(
        "UPDATE publications SET status = $2, data_level_max = $3, quota_rpm = $4, \
         quota_tpm = $5, quota_daily_requests = $6, note = $7, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(&fields.status)
    .bind(fields.data_level_max)
    .bind(fields.quota_rpm)
    .bind(fields.quota_tpm)
    .bind(fields.quota_daily_requests)
    .bind(&fields.note)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn delete_publication(conn: &mut PgConnection, id: Uuid) -> Result<bool, AppError> {
    let affected = sqlx::query("DELETE FROM publications WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

fn map_publication_write_error(error: sqlx::Error) -> AppError {
    match constraint_name(&error) {
        Some("publications_model_group_key") => AppError::conflict(
            "publication_conflict",
            "该模型组已有发布（一个模型组只能有一条发布）",
            None,
        ),
        Some("publications_model_group_id_fkey") => {
            AppError::not_found("model_group_not_found", "模型组不存在")
        }
        Some("publication_targets_publication_id_fkey") => {
            AppError::not_found("publication_not_found", "发布不存在")
        }
        Some(_) | None => AppError::from(error),
    }
}

// ===========================================================================
// 工具默认模型（Tool Model Default，迭代 4）
// ===========================================================================

const TOOL_DEFAULT_COLUMNS: &str = "t.id, t.tool_key, t.scope_type, t.scope_id, \
     t.default_model_group_id, t.allow_switch, g.client_model_name AS default_model_client_name, \
     t.created_at, t.updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ToolDefaultRow {
    pub id: Uuid,
    pub tool_key: String,
    pub scope_type: String,
    pub scope_id: Option<Uuid>,
    pub default_model_group_id: Uuid,
    pub default_model_client_name: String,
    pub allow_switch: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ToolDefaultModelRow {
    pub tool_default_id: Uuid,
    pub model_group_id: Uuid,
}

/// 投影读取用的工具默认（不含 join 名称）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectionToolDefaultRow {
    pub id: Uuid,
    pub tool_key: String,
    pub scope_type: String,
    pub scope_id: Option<Uuid>,
    pub default_model_group_id: Uuid,
    pub allow_switch: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectionToolDefaultModelRow {
    pub tool_default_id: Uuid,
    pub model_group_id: Uuid,
}

pub struct ToolDefaultFilters {
    pub tool_key: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<Uuid>,
    pub limit: i64,
    pub offset: i64,
}

fn tool_default_where_clause() -> &'static str {
    "WHERE ($1::text IS NULL OR t.tool_key = $1) \
       AND ($2::text IS NULL OR t.scope_type = $2) \
       AND ($3::uuid IS NULL OR t.scope_id = $3)"
}

pub async fn list_tool_model_defaults(
    pool: &PgPool,
    filters: &ToolDefaultFilters,
) -> Result<(Vec<ToolDefaultRow>, i64), AppError> {
    let where_clause = tool_default_where_clause();
    let rows = sqlx::query_as::<_, ToolDefaultRow>(&format!(
        "SELECT {TOOL_DEFAULT_COLUMNS} FROM tool_model_defaults t \
         JOIN model_groups g ON g.id = t.default_model_group_id {where_clause} \
         ORDER BY t.created_at DESC, t.id DESC LIMIT $4 OFFSET $5"
    ))
    .bind(filters.tool_key.as_deref())
    .bind(filters.scope_type.as_deref())
    .bind(filters.scope_id)
    .bind(filters.limit)
    .bind(filters.offset)
    .fetch_all(pool)
    .await?;
    let total = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT count(*) FROM tool_model_defaults t {where_clause}"
    ))
    .bind(filters.tool_key.as_deref())
    .bind(filters.scope_type.as_deref())
    .bind(filters.scope_id)
    .fetch_one(pool)
    .await?;
    Ok((rows, total))
}

pub async fn list_tool_default_models_for_defaults(
    pool: &PgPool,
    tool_default_ids: &[Uuid],
) -> Result<Vec<ToolDefaultModelRow>, AppError> {
    if tool_default_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, ToolDefaultModelRow>(
        "SELECT tool_default_id, model_group_id FROM tool_model_default_models \
         WHERE tool_default_id = ANY($1) ORDER BY tool_default_id, model_group_id",
    )
    .bind(tool_default_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_tool_default_models_conn(
    conn: &mut PgConnection,
    tool_default_id: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    let rows = sqlx::query_scalar::<_, Uuid>(
        "SELECT model_group_id FROM tool_model_default_models \
         WHERE tool_default_id = $1 ORDER BY model_group_id",
    )
    .bind(tool_default_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

pub async fn get_tool_model_default(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<ToolDefaultRow>, AppError> {
    let row = sqlx::query_as::<_, ToolDefaultRow>(&format!(
        "SELECT {TOOL_DEFAULT_COLUMNS} FROM tool_model_defaults t \
         JOIN model_groups g ON g.id = t.default_model_group_id WHERE t.id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_tool_model_default_for_update(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<ToolDefaultRow>, AppError> {
    let row = sqlx::query_as::<_, ToolDefaultRow>(&format!(
        "SELECT {TOOL_DEFAULT_COLUMNS} FROM tool_model_defaults t \
         JOIN model_groups g ON g.id = t.default_model_group_id \
         WHERE t.id = $1 FOR UPDATE OF t"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

pub async fn insert_tool_default(
    conn: &mut PgConnection,
    id: Uuid,
    fields: &super::model::ToolDefaultFields,
) -> Result<(), AppError> {
    let result = sqlx::query(
        "INSERT INTO tool_model_defaults \
         (id, tool_key, scope_type, scope_id, default_model_group_id, allow_switch) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(&fields.tool_key)
    .bind(&fields.scope_type)
    .bind(fields.scope_id)
    .bind(fields.default_model_group_id)
    .bind(fields.allow_switch)
    .execute(&mut *conn)
    .await;
    result.map_err(map_tool_default_write_error)?;
    replace_tool_default_models(conn, id, &fields.allowed_model_group_ids).await
}

pub async fn replace_tool_default_models(
    conn: &mut PgConnection,
    tool_default_id: Uuid,
    model_group_ids: &[Uuid],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM tool_model_default_models WHERE tool_default_id = $1")
        .bind(tool_default_id)
        .execute(&mut *conn)
        .await?;
    for model_group_id in model_group_ids {
        sqlx::query(
            "INSERT INTO tool_model_default_models (id, tool_default_id, model_group_id) \
             VALUES ($1, $2, $3)",
        )
        .bind(Uuid::new_v4())
        .bind(tool_default_id)
        .bind(model_group_id)
        .execute(&mut *conn)
        .await
        .map_err(map_tool_default_write_error)?;
    }
    Ok(())
}

pub async fn update_tool_default(
    conn: &mut PgConnection,
    id: Uuid,
    fields: &super::model::ToolDefaultFields,
) -> Result<bool, AppError> {
    let affected = sqlx::query(
        "UPDATE tool_model_defaults SET default_model_group_id = $2, allow_switch = $3, \
         updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(fields.default_model_group_id)
    .bind(fields.allow_switch)
    .execute(&mut *conn)
    .await
    .map_err(map_tool_default_write_error)?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn delete_tool_default(conn: &mut PgConnection, id: Uuid) -> Result<bool, AppError> {
    let affected = sqlx::query("DELETE FROM tool_model_defaults WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

fn map_tool_default_write_error(error: sqlx::Error) -> AppError {
    match constraint_name(&error) {
        Some("tool_model_defaults_scope_key") | Some("tool_model_defaults_global_key") => {
            AppError::conflict(
                "tool_model_default_conflict",
                "该工具在该作用域已有默认模型",
                None,
            )
        }
        Some("tool_model_defaults_default_model_group_id_fkey")
        | Some("tool_model_default_models_model_group_id_fkey") => {
            AppError::not_found("model_group_not_found", "模型组不存在")
        }
        Some(_) | None => AppError::from(error),
    }
}

// ===========================================================================
// 组详情聚合（迭代 4 顺带：publication / tool_default 引用计数）
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GroupPublicationRow {
    pub model_group_id: Uuid,
    pub id: Uuid,
    pub status: String,
    pub target_count: i64,
    pub updated_at: DateTime<Utc>,
}

pub async fn list_group_publications(
    pool: &PgPool,
    group_ids: &[Uuid],
) -> Result<Vec<GroupPublicationRow>, AppError> {
    if group_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, GroupPublicationRow>(
        "SELECT p.model_group_id, p.id, p.status, p.updated_at, \
         (SELECT count(*) FROM publication_targets t WHERE t.publication_id = p.id) AS target_count \
         FROM publications p WHERE p.model_group_id = ANY($1)",
    )
    .bind(group_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count_tool_defaults_by_group(
    pool: &PgPool,
    group_ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, AppError> {
    if group_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_as::<_, (Uuid, i64)>(
        "SELECT model_group_id, count(DISTINCT tool_default_id) FROM ( \
           SELECT default_model_group_id AS model_group_id, id AS tool_default_id \
             FROM tool_model_defaults \
           UNION ALL \
           SELECT model_group_id, tool_default_id FROM tool_model_default_models \
         ) refs WHERE model_group_id = ANY($1) GROUP BY model_group_id",
    )
    .bind(group_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

// ===========================================================================
// 聚合建议（补全）：未入组部署 + 决策表
// ===========================================================================

/// 未入组部署（聚合建议候选），按创建时间稳定排序。
pub async fn list_ungrouped_deployments(pool: &PgPool) -> Result<Vec<DeploymentRow>, AppError> {
    let rows = sqlx::query_as::<_, DeploymentRow>(&format!(
        "SELECT {DEPLOYMENT_COLUMNS} FROM deployments \
         WHERE model_group_id IS NULL ORDER BY created_at, id"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MergeDecisionRow {
    pub suggestion_key: String,
    pub decision: String,
    pub model_group_id: Option<Uuid>,
    pub deployment_ids: Vec<Uuid>,
    pub decided_by: Option<Uuid>,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

/// 新决策的插入参数。
pub struct NewMergeDecision<'a> {
    pub suggestion_key: &'a str,
    pub canonical: &'a str,
    pub upstream_model_name: &'a str,
    pub decision: &'a str,
    pub model_group_id: Option<Uuid>,
    pub deployment_ids: &'a [Uuid],
    pub diff: &'a serde_json::Value,
    pub decided_by: Option<Uuid>,
    pub note: &'a str,
}

pub async fn get_merge_decision(
    pool: &PgPool,
    suggestion_key: &str,
) -> Result<Option<MergeDecisionRow>, AppError> {
    let row = sqlx::query_as::<_, MergeDecisionRow>(
        "SELECT suggestion_key, decision, model_group_id, deployment_ids, decided_by, note, \
         created_at FROM model_group_merge_decisions WHERE suggestion_key = $1",
    )
    .bind(suggestion_key)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn rejected_suggestion_keys(
    pool: &PgPool,
    suggestion_keys: &[String],
) -> Result<std::collections::HashSet<String>, AppError> {
    if suggestion_keys.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT suggestion_key FROM model_group_merge_decisions \
         WHERE decision = 'rejected' AND suggestion_key = ANY($1)",
    )
    .bind(suggestion_keys)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// 插入决策；`suggestion_key` 唯一约束冲突时返回 `false`（并发兜底）。
pub async fn insert_merge_decision(
    conn: &mut PgConnection,
    decision: &NewMergeDecision<'_>,
) -> Result<bool, AppError> {
    let affected = sqlx::query(
        "INSERT INTO model_group_merge_decisions \
         (id, suggestion_key, canonical, upstream_model_name, decision, model_group_id, \
          deployment_ids, diff, decided_by, note) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         ON CONFLICT (suggestion_key) DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(decision.suggestion_key)
    .bind(decision.canonical)
    .bind(decision.upstream_model_name)
    .bind(decision.decision)
    .bind(decision.model_group_id)
    .bind(decision.deployment_ids)
    .bind(decision.diff)
    .bind(decision.decided_by)
    .bind(decision.note)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// 把未入组部署挂到模型组；返回实际更新行数（调用方校验是否等于期望数）。
pub async fn attach_deployments(
    conn: &mut PgConnection,
    deployment_ids: &[Uuid],
    model_group_id: Uuid,
) -> Result<u64, AppError> {
    if deployment_ids.is_empty() {
        return Ok(0);
    }
    let affected = sqlx::query(
        "UPDATE deployments SET model_group_id = $2, updated_at = now() \
         WHERE id = ANY($1) AND model_group_id IS NULL",
    )
    .bind(deployment_ids)
    .bind(model_group_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(affected)
}

/// 同 canonical 的既有模型组（供 merge_into_group 选择；归档组也返回由调用方过滤）。
pub async fn list_groups_by_canonical(
    pool: &PgPool,
    canonical: &str,
    limit: i64,
) -> Result<Vec<ModelGroupRow>, AppError> {
    let rows = sqlx::query_as::<_, ModelGroupRow>(&format!(
        "SELECT {GROUP_COLUMNS} FROM model_groups WHERE canonical = $1 \
         ORDER BY created_at, id LIMIT $2"
    ))
    .bind(canonical)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
