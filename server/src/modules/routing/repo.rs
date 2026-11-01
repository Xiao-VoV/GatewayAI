//! routing 数据访问（表归属：`model_aliases` / `model_alias_targets`）。

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::platform::error::AppError;

use super::model::TargetSpec;

#[derive(Debug, sqlx::FromRow)]
pub struct AliasRow {
    pub id: Uuid,
    pub alias: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct TargetRow {
    pub id: Uuid,
    pub alias_id: Uuid,
    pub upstream_model_id: Uuid,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const ALIAS_COLUMNS: &str = "id, alias, enabled, created_at, updated_at";
const TARGET_COLUMNS: &str =
    "id, alias_id, upstream_model_id, priority, weight, enabled, created_at, updated_at";

fn map_write_error(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(database_error) = &error {
        match database_error.code().as_deref() {
            Some("23505") => {
                return AppError::conflict("model_alias_conflict", "模型别名已存在", None);
            }
            Some("23503") => {
                return AppError::not_found("upstream_model_not_found", "上游模型不存在");
            }
            _ => {}
        }
    }
    AppError::from(error)
}

pub async fn list_aliases(
    pool: &PgPool,
    enabled: Option<bool>,
    q: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AliasRow>, i64), AppError> {
    let rows = sqlx::query_as::<_, AliasRow>(&format!(
        "SELECT {ALIAS_COLUMNS} FROM model_aliases \
         WHERE ($1::boolean IS NULL OR enabled = $1) \
         AND ($2::text IS NULL OR alias ILIKE '%' || $2 || '%') \
         ORDER BY created_at DESC, id DESC LIMIT $3 OFFSET $4"
    ))
    .bind(enabled)
    .bind(q)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM model_aliases \
         WHERE ($1::boolean IS NULL OR enabled = $1) \
         AND ($2::text IS NULL OR alias ILIKE '%' || $2 || '%')",
    )
    .bind(enabled)
    .bind(q)
    .fetch_one(pool)
    .await?;
    Ok((rows, total))
}

pub async fn get_alias(pool: &PgPool, id: Uuid) -> Result<Option<AliasRow>, AppError> {
    let row = sqlx::query_as::<_, AliasRow>(&format!(
        "SELECT {ALIAS_COLUMNS} FROM model_aliases WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_targets(pool: &PgPool, alias_id: Uuid) -> Result<Vec<TargetRow>, AppError> {
    let rows = sqlx::query_as::<_, TargetRow>(&format!(
        "SELECT {TARGET_COLUMNS} FROM model_alias_targets WHERE alias_id = $1 \
         ORDER BY priority ASC, id ASC"
    ))
    .bind(alias_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn all_aliases(pool: &PgPool) -> Result<Vec<AliasRow>, AppError> {
    let rows = sqlx::query_as::<_, AliasRow>(&format!(
        "SELECT {ALIAS_COLUMNS} FROM model_aliases ORDER BY alias ASC"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn all_targets(pool: &PgPool) -> Result<Vec<TargetRow>, AppError> {
    let rows = sqlx::query_as::<_, TargetRow>(&format!(
        "SELECT {TARGET_COLUMNS} FROM model_alias_targets ORDER BY alias_id, priority, id"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn insert_targets(
    conn: &mut PgConnection,
    alias_id: Uuid,
    targets: &[TargetSpec],
) -> Result<(), AppError> {
    for target in targets {
        sqlx::query(
            "INSERT INTO model_alias_targets \
             (id, alias_id, upstream_model_id, priority, weight, enabled) \
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5)",
        )
        .bind(alias_id)
        .bind(target.upstream_model_id)
        .bind(target.priority)
        .bind(target.weight)
        .bind(target.enabled)
        .execute(&mut *conn)
        .await
        .map_err(map_write_error)?;
    }
    Ok(())
}

pub async fn create_alias(
    pool: &PgPool,
    id: Uuid,
    alias: &str,
    enabled: bool,
    targets: &[TargetSpec],
) -> Result<AliasRow, AppError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, AliasRow>(&format!(
        "INSERT INTO model_aliases (id, alias, enabled) VALUES ($1, $2, $3) \
         RETURNING {ALIAS_COLUMNS}"
    ))
    .bind(id)
    .bind(alias)
    .bind(enabled)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_write_error)?;
    insert_targets(&mut tx, row.id, targets).await?;
    tx.commit().await?;
    Ok(row)
}

/// 返回 `None` 表示别名不存在；targets `None` = 不替换。
pub async fn update_alias(
    pool: &PgPool,
    id: Uuid,
    alias: &str,
    enabled: bool,
    targets: Option<&[TargetSpec]>,
) -> Result<Option<AliasRow>, AppError> {
    let mut tx = pool.begin().await?;
    let locked =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM model_aliases WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if locked.is_none() {
        tx.rollback().await.ok();
        return Ok(None);
    }
    let row = sqlx::query_as::<_, AliasRow>(&format!(
        "UPDATE model_aliases SET alias = $2, enabled = $3, updated_at = now() \
         WHERE id = $1 RETURNING {ALIAS_COLUMNS}"
    ))
    .bind(id)
    .bind(alias)
    .bind(enabled)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_write_error)?;
    if let Some(targets) = targets {
        sqlx::query("DELETE FROM model_alias_targets WHERE alias_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        insert_targets(&mut tx, id, targets).await?;
    }
    tx.commit().await?;
    Ok(Some(row))
}

pub async fn delete_alias(pool: &PgPool, id: Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM model_aliases WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
