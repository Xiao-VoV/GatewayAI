//! policy 数据访问（表归属：`budgets`）。

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::platform::error::AppError;

#[derive(Debug, sqlx::FromRow)]
pub struct BudgetRow {
    pub id: Uuid,
    pub user_id: Uuid,
    /// NUMERIC(18,6) 的文本形式，避免驱动浮点。
    pub daily_limit: String,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const SELECT: &str = "SELECT id, user_id, daily_limit::text AS daily_limit, currency, \
    created_at, updated_at FROM budgets";

pub async fn list(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<(Vec<BudgetRow>, i64), AppError> {
    let rows = sqlx::query_as::<_, BudgetRow>(&format!(
        "{SELECT} ORDER BY updated_at DESC, id DESC LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    let total = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM budgets")
        .fetch_one(pool)
        .await?;
    Ok((rows, total))
}

pub async fn get(pool: &PgPool, user_id: Uuid) -> Result<Option<BudgetRow>, AppError> {
    let row = sqlx::query_as::<_, BudgetRow>(&format!("{SELECT} WHERE user_id = $1"))
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn upsert(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    daily_limit: &str,
    currency: &str,
) -> Result<BudgetRow, AppError> {
    let row = sqlx::query_as::<_, BudgetRow>(
        "INSERT INTO budgets (id, user_id, daily_limit, currency) \
         VALUES ($1, $2, $3::text::numeric, $4) \
         ON CONFLICT (user_id) DO UPDATE SET \
           daily_limit = EXCLUDED.daily_limit, currency = EXCLUDED.currency, updated_at = now() \
         RETURNING id, user_id, daily_limit::text AS daily_limit, currency, created_at, updated_at",
    )
    .bind(id)
    .bind(user_id)
    .bind(daily_limit)
    .bind(currency)
    .fetch_one(pool)
    .await
    .map_err(map_budget_error)?;
    Ok(row)
}

fn map_budget_error(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(database_error) = &error
        && database_error.code().as_deref() == Some("23503")
    {
        return AppError::not_found("user_not_found", "员工用户不存在");
    }
    AppError::from(error)
}

pub async fn delete(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM budgets WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
