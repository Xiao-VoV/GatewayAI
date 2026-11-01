//! providers 的 SQLx 数据访问（表归属：`providers` / `provider_accounts` / `upstream_models`）。

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::platform::error::AppError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProviderRow {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub description: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountRow {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub name: String,
    pub region: String,
    pub credential_encrypted: Vec<u8>,
    pub credential_hint: String,
    pub weight: i32,
    pub priority: i32,
    pub rpm_limit: Option<i32>,
    pub tpm_limit: Option<i64>,
    pub health: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ModelRow {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub name: String,
    pub canonical: String,
    pub capabilities: serde_json::Value,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct RoutingUpstreamRow {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub name: String,
    pub canonical: String,
    pub capabilities: serde_json::Value,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ProviderStatusRow {
    pub provider_id: Uuid,
    pub status: String,
    pub active_account_count: i64,
}

/// proxy 账号选择的最小账号快照（不含凭证/名称，避免无谓读取密文）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SelectableAccountRow {
    pub id: Uuid,
    pub priority: i32,
    pub weight: i32,
    pub health: String,
    pub created_at: DateTime<Utc>,
}

pub async fn routing_upstream_rows(pool: &PgPool) -> Result<Vec<RoutingUpstreamRow>, AppError> {
    let rows = sqlx::query_as::<_, RoutingUpstreamRow>(
        "SELECT m.id, m.provider_id, p.name AS provider_name, p.kind AS provider_kind, \
         p.base_url, m.name, m.canonical, m.capabilities, m.context_window, m.max_output, \
         m.price_input::float8 AS price_input, m.price_output::float8 AS price_output, m.currency \
         FROM upstream_models m JOIN providers p ON p.id = m.provider_id ORDER BY m.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn provider_status_rows(
    pool: &PgPool,
    provider_ids: &[Uuid],
) -> Result<Vec<ProviderStatusRow>, AppError> {
    if provider_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, ProviderStatusRow>(
        "SELECT p.id AS provider_id, p.status, \
         (SELECT count(*) FROM provider_accounts a \
          WHERE a.provider_id = p.id AND a.status = 'active') AS active_account_count \
         FROM providers p WHERE p.id = ANY($1)",
    )
    .bind(provider_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, sqlx::FromRow)]
pub struct ProviderCounts {
    pub account_count: i64,
    pub active_account_count: i64,
    pub upstream_model_count: i64,
    pub model_alias_count: i64,
    pub deployment_count: i64,
}

pub struct NewProvider {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub description: String,
}

pub struct NewAccount {
    pub id: Uuid,
    pub name: String,
    pub region: String,
    pub credential_encrypted: Vec<u8>,
    pub credential_hint: String,
    pub weight: i32,
    pub priority: i32,
    pub rpm_limit: Option<i32>,
    pub tpm_limit: Option<i64>,
}

#[derive(Default)]
pub struct ProviderPatch {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
}

#[derive(Default)]
pub struct AccountPatch {
    pub name: Option<String>,
    pub region: Option<String>,
    pub credential_encrypted: Option<Vec<u8>>,
    pub credential_hint: Option<String>,
    pub weight: Option<i32>,
    pub priority: Option<i32>,
    pub rpm_limit: Option<i32>,
    pub tpm_limit: Option<i64>,
    pub status: Option<String>,
}

pub struct NewModel {
    pub id: Uuid,
    pub name: String,
    pub canonical: String,
    pub capabilities: serde_json::Value,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
}

#[derive(Default)]
pub struct ModelPatch {
    pub name: Option<String>,
    pub canonical: Option<String>,
    pub capabilities: Option<serde_json::Value>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: Option<String>,
}

const PROVIDER_COLUMNS: &str =
    "id, name, kind, base_url, description, status, created_at, updated_at";
const ACCOUNT_COLUMNS: &str = "id, provider_id, name, region, credential_encrypted, credential_hint, \
     weight, priority, rpm_limit, tpm_limit, health, last_checked_at, last_error, status, \
     created_at, updated_at";
const MODEL_COLUMNS: &str = "id, provider_id, name, canonical, capabilities, context_window, \
     max_output, price_input::float8 AS price_input, price_output::float8 AS price_output, \
     currency, created_at, updated_at";

pub async fn count_providers(
    pool: &PgPool,
    status: Option<&str>,
    kind: Option<&str>,
) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM providers \
         WHERE ($1::text IS NULL OR status = $1) AND ($2::text IS NULL OR kind = $2)",
    )
    .bind(status)
    .bind(kind)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn list_providers(
    pool: &PgPool,
    status: Option<&str>,
    kind: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ProviderRow>, AppError> {
    let sql = format!(
        "SELECT {PROVIDER_COLUMNS} FROM providers \
         WHERE ($1::text IS NULL OR status = $1) AND ($2::text IS NULL OR kind = $2) \
         ORDER BY created_at DESC, id DESC LIMIT $3 OFFSET $4"
    );
    let rows = sqlx::query_as::<_, ProviderRow>(&sql)
        .bind(status)
        .bind(kind)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn get_provider(pool: &PgPool, id: Uuid) -> Result<Option<ProviderRow>, AppError> {
    let sql = format!("SELECT {PROVIDER_COLUMNS} FROM providers WHERE id = $1");
    Ok(sqlx::query_as::<_, ProviderRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn provider_counts(pool: &PgPool, id: Uuid) -> Result<Option<ProviderCounts>, AppError> {
    let row = sqlx::query_as::<_, ProviderCounts>(
        "SELECT \
           (SELECT count(*) FROM provider_accounts a WHERE a.provider_id = p.id) AS account_count, \
           (SELECT count(*) FROM provider_accounts a WHERE a.provider_id = p.id AND a.status = 'active') AS active_account_count, \
           (SELECT count(*) FROM upstream_models m WHERE m.provider_id = p.id) AS upstream_model_count, \
           (SELECT count(DISTINCT t.alias_id) FROM model_alias_targets t \
              JOIN upstream_models m ON m.id = t.upstream_model_id \
              WHERE m.provider_id = p.id) AS model_alias_count, \
           (SELECT count(*) FROM deployments d \
              JOIN upstream_models m ON m.id = d.upstream_model_id \
              JOIN provider_accounts a ON a.id = d.provider_account_id \
              WHERE m.provider_id = p.id OR a.provider_id = p.id) AS deployment_count \
         FROM providers p WHERE p.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 创建 provider + 首个账号（同事务）。
pub async fn create_provider_with_account(
    pool: &PgPool,
    provider: NewProvider,
    account: NewAccount,
) -> Result<(ProviderRow, AccountRow), AppError> {
    let mut tx = pool.begin().await?;
    let insert_provider = sqlx::query(
        "INSERT INTO providers (id, name, kind, base_url, description, status) \
         VALUES ($1, $2, $3, $4, $5, 'active')",
    )
    .bind(provider.id)
    .bind(&provider.name)
    .bind(&provider.kind)
    .bind(&provider.base_url)
    .bind(&provider.description)
    .execute(&mut *tx)
    .await;
    if let Err(error) = insert_provider {
        return Err(map_unique_conflict(
            error,
            "provider_name_conflict",
            "供应商名称已存在",
        ));
    }

    let insert_account = sqlx::query(
        "INSERT INTO provider_accounts \
         (id, provider_id, name, region, credential_encrypted, credential_hint, weight, priority, rpm_limit, tpm_limit) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(account.id)
    .bind(provider.id)
    .bind(&account.name)
    .bind(&account.region)
    .bind(&account.credential_encrypted)
    .bind(&account.credential_hint)
    .bind(account.weight)
    .bind(account.priority)
    .bind(account.rpm_limit)
    .bind(account.tpm_limit)
    .execute(&mut *tx)
    .await;
    if let Err(error) = insert_account {
        return Err(map_unique_conflict(
            error,
            "account_name_conflict",
            "该供应商下账号名称已存在",
        ));
    }

    let provider_sql = format!("SELECT {PROVIDER_COLUMNS} FROM providers WHERE id = $1");
    let provider_row = sqlx::query_as::<_, ProviderRow>(&provider_sql)
        .bind(provider.id)
        .fetch_one(&mut *tx)
        .await?;
    let account_sql = format!("SELECT {ACCOUNT_COLUMNS} FROM provider_accounts WHERE id = $1");
    let account_row = sqlx::query_as::<_, AccountRow>(&account_sql)
        .bind(account.id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((provider_row, account_row))
}

pub async fn update_provider(
    pool: &PgPool,
    id: Uuid,
    patch: ProviderPatch,
) -> Result<Option<ProviderRow>, AppError> {
    let sql = format!(
        "UPDATE providers SET \
           name = COALESCE($2, name), \
           base_url = COALESCE($3, base_url), \
           description = COALESCE($4, description), \
           status = COALESCE($5, status), \
           updated_at = now() \
         WHERE id = $1 RETURNING {PROVIDER_COLUMNS}"
    );
    let result = sqlx::query_as::<_, ProviderRow>(&sql)
        .bind(id)
        .bind(patch.name)
        .bind(patch.base_url)
        .bind(patch.description)
        .bind(patch.status)
        .fetch_optional(pool)
        .await;
    match result {
        Ok(row) => Ok(row),
        Err(error) => Err(map_unique_conflict(
            error,
            "provider_name_conflict",
            "供应商名称已存在",
        )),
    }
}

pub async fn delete_provider(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM providers WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await;
    match result {
        Ok(result) => Ok(result.rows_affected() > 0),
        Err(error) => Err(map_fk_conflict(
            error,
            "provider_in_use",
            "供应商仍被引用，不能删除；请先解除引用或禁用",
        )),
    }
}

pub async fn list_accounts(pool: &PgPool, provider_id: Uuid) -> Result<Vec<AccountRow>, AppError> {
    let sql = format!(
        "SELECT {ACCOUNT_COLUMNS} FROM provider_accounts WHERE provider_id = $1 \
         ORDER BY priority ASC, created_at ASC"
    );
    Ok(sqlx::query_as::<_, AccountRow>(&sql)
        .bind(provider_id)
        .fetch_all(pool)
        .await?)
}

pub async fn get_account(
    pool: &PgPool,
    provider_id: Uuid,
    account_id: Uuid,
) -> Result<Option<AccountRow>, AppError> {
    let sql = format!(
        "SELECT {ACCOUNT_COLUMNS} FROM provider_accounts WHERE id = $1 AND provider_id = $2"
    );
    Ok(sqlx::query_as::<_, AccountRow>(&sql)
        .bind(account_id)
        .bind(provider_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn insert_account(
    pool: &PgPool,
    provider_id: Uuid,
    account: NewAccount,
) -> Result<AccountRow, AppError> {
    let sql = format!(
        "INSERT INTO provider_accounts \
         (id, provider_id, name, region, credential_encrypted, credential_hint, weight, priority, rpm_limit, tpm_limit) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING {ACCOUNT_COLUMNS}"
    );
    let result = sqlx::query_as::<_, AccountRow>(&sql)
        .bind(account.id)
        .bind(provider_id)
        .bind(&account.name)
        .bind(&account.region)
        .bind(&account.credential_encrypted)
        .bind(&account.credential_hint)
        .bind(account.weight)
        .bind(account.priority)
        .bind(account.rpm_limit)
        .bind(account.tpm_limit)
        .fetch_one(pool)
        .await;
    match result {
        Ok(row) => Ok(row),
        Err(error) => Err(map_unique_conflict(
            error,
            "account_name_conflict",
            "该供应商下账号名称已存在",
        )),
    }
}

pub async fn update_account(
    pool: &PgPool,
    provider_id: Uuid,
    account_id: Uuid,
    patch: AccountPatch,
) -> Result<Option<AccountRow>, AppError> {
    let sql = format!(
        "UPDATE provider_accounts SET \
           name = COALESCE($3, name), \
           region = COALESCE($4, region), \
           credential_encrypted = COALESCE($5, credential_encrypted), \
           credential_hint = COALESCE($6, credential_hint), \
           weight = COALESCE($7, weight), \
           priority = COALESCE($8, priority), \
           rpm_limit = COALESCE($9, rpm_limit), \
           tpm_limit = COALESCE($10, tpm_limit), \
           status = COALESCE($11, status), \
           updated_at = now() \
         WHERE id = $1 AND provider_id = $2 RETURNING {ACCOUNT_COLUMNS}"
    );
    let result = sqlx::query_as::<_, AccountRow>(&sql)
        .bind(account_id)
        .bind(provider_id)
        .bind(patch.name)
        .bind(patch.region)
        .bind(patch.credential_encrypted)
        .bind(patch.credential_hint)
        .bind(patch.weight)
        .bind(patch.priority)
        .bind(patch.rpm_limit)
        .bind(patch.tpm_limit)
        .bind(patch.status)
        .fetch_optional(pool)
        .await;
    match result {
        Ok(row) => Ok(row),
        Err(error) => Err(map_unique_conflict(
            error,
            "account_name_conflict",
            "该供应商下账号名称已存在",
        )),
    }
}

pub async fn delete_account(
    pool: &PgPool,
    provider_id: Uuid,
    account_id: Uuid,
) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM provider_accounts WHERE id = $1 AND provider_id = $2")
        .bind(account_id)
        .bind(provider_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// 测试用账号：指定 account_id 时校验归属；否则取第一个 active（priority 最小、创建最早）。
pub async fn account_for_test(
    pool: &PgPool,
    provider_id: Uuid,
    account_id: Option<Uuid>,
) -> Result<Option<AccountRow>, AppError> {
    if let Some(account_id) = account_id {
        return get_account(pool, provider_id, account_id).await;
    }
    let sql = format!(
        "SELECT {ACCOUNT_COLUMNS} FROM provider_accounts \
         WHERE provider_id = $1 AND status = 'active' \
         ORDER BY priority ASC, created_at ASC LIMIT 1"
    );
    Ok(sqlx::query_as::<_, AccountRow>(&sql)
        .bind(provider_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn update_health(
    pool: &PgPool,
    account_id: Uuid,
    healthy: bool,
    last_error: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE provider_accounts SET health = $2, last_checked_at = now(), last_error = $3 \
         WHERE id = $1",
    )
    .bind(account_id)
    .bind(if healthy { "healthy" } else { "unhealthy" })
    .bind(last_error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_models(pool: &PgPool, provider_id: Uuid) -> Result<Vec<ModelRow>, AppError> {
    let sql = format!(
        "SELECT {MODEL_COLUMNS} FROM upstream_models WHERE provider_id = $1 ORDER BY name ASC"
    );
    Ok(sqlx::query_as::<_, ModelRow>(&sql)
        .bind(provider_id)
        .fetch_all(pool)
        .await?)
}

pub async fn deployment_count_for_account(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM deployments WHERE provider_account_id = $1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn deployment_count_for_model(pool: &PgPool, model_id: Uuid) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM deployments WHERE upstream_model_id = $1",
    )
    .bind(model_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn model_alias_count(pool: &PgPool, model_id: Uuid) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(DISTINCT alias_id) FROM model_alias_targets WHERE upstream_model_id = $1",
    )
    .bind(model_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn insert_model(
    pool: &PgPool,
    provider_id: Uuid,
    model: NewModel,
) -> Result<ModelRow, AppError> {
    let sql = format!(
        "INSERT INTO upstream_models \
         (id, provider_id, name, canonical, capabilities, context_window, max_output, price_input, price_output, currency) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::numeric, $9::text::numeric, $10) \
         RETURNING {MODEL_COLUMNS}"
    );
    let result = sqlx::query_as::<_, ModelRow>(&sql)
        .bind(model.id)
        .bind(provider_id)
        .bind(&model.name)
        .bind(&model.canonical)
        .bind(&model.capabilities)
        .bind(model.context_window)
        .bind(model.max_output)
        .bind(decimal_bind(model.price_input))
        .bind(decimal_bind(model.price_output))
        .bind(&model.currency)
        .fetch_one(pool)
        .await;
    match result {
        Ok(row) => Ok(row),
        Err(error) => Err(map_unique_conflict(
            error,
            "upstream_model_conflict",
            "该供应商下上游模型名已存在",
        )),
    }
}

pub async fn update_model(
    pool: &PgPool,
    provider_id: Uuid,
    model_id: Uuid,
    patch: ModelPatch,
) -> Result<Option<ModelRow>, AppError> {
    let sql = format!(
        "UPDATE upstream_models SET \
           name = COALESCE($3, name), \
           canonical = COALESCE($4, canonical), \
           capabilities = COALESCE($5, capabilities), \
           context_window = COALESCE($6, context_window), \
           max_output = COALESCE($7, max_output), \
           price_input = COALESCE($8::text::numeric, price_input), \
           price_output = COALESCE($9::text::numeric, price_output), \
           currency = COALESCE($10, currency), \
           updated_at = now() \
         WHERE id = $1 AND provider_id = $2 RETURNING {MODEL_COLUMNS}"
    );
    let result = sqlx::query_as::<_, ModelRow>(&sql)
        .bind(model_id)
        .bind(provider_id)
        .bind(patch.name)
        .bind(patch.canonical)
        .bind(patch.capabilities)
        .bind(patch.context_window)
        .bind(patch.max_output)
        .bind(decimal_bind(patch.price_input))
        .bind(decimal_bind(patch.price_output))
        .bind(patch.currency)
        .fetch_optional(pool)
        .await;
    match result {
        Ok(row) => Ok(row),
        Err(error) => Err(map_unique_conflict(
            error,
            "upstream_model_conflict",
            "该供应商下上游模型名已存在",
        )),
    }
}

pub async fn delete_model(
    pool: &PgPool,
    provider_id: Uuid,
    model_id: Uuid,
) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM upstream_models WHERE id = $1 AND provider_id = $2")
        .bind(model_id)
        .bind(provider_id)
        .execute(pool)
        .await;
    match result {
        Ok(result) => Ok(result.rows_affected() > 0),
        Err(error) => Err(map_fk_conflict(
            error,
            "upstream_model_in_use",
            "上游模型仍被模型别名引用，不能删除",
        )),
    }
}

/// 内部接口：provider 下可参与选择的 active 账号（不含凭证）。
pub async fn selectable_accounts(
    pool: &PgPool,
    provider_id: Uuid,
) -> Result<Vec<SelectableAccountRow>, AppError> {
    Ok(sqlx::query_as::<_, SelectableAccountRow>(
        "SELECT id, priority, weight, health, created_at FROM provider_accounts \
         WHERE provider_id = $1 AND status = 'active' \
         ORDER BY priority ASC, created_at ASC, id ASC",
    )
    .bind(provider_id)
    .fetch_all(pool)
    .await?)
}

/// 内部接口：解密用账号行 + provider 是否 active。
pub async fn account_for_decrypt(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<Option<(AccountRow, bool)>, AppError> {
    #[derive(sqlx::FromRow)]
    struct RowWithFlag {
        id: Uuid,
        provider_id: Uuid,
        name: String,
        region: String,
        credential_encrypted: Vec<u8>,
        credential_hint: String,
        weight: i32,
        priority: i32,
        rpm_limit: Option<i32>,
        tpm_limit: Option<i64>,
        health: String,
        last_checked_at: Option<DateTime<Utc>>,
        last_error: Option<String>,
        status: String,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        provider_active: bool,
    }

    let row = sqlx::query_as::<_, RowWithFlag>(
        "SELECT a.id, a.provider_id, a.name, a.region, a.credential_encrypted, a.credential_hint, \
                a.weight, a.priority, a.rpm_limit, a.tpm_limit, a.health, a.last_checked_at, a.last_error, \
                a.status, a.created_at, a.updated_at, p.status = 'active' AS provider_active \
         FROM provider_accounts a JOIN providers p ON p.id = a.provider_id \
         WHERE a.id = $1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| {
        (
            AccountRow {
                id: row.id,
                provider_id: row.provider_id,
                name: row.name,
                region: row.region,
                credential_encrypted: row.credential_encrypted,
                credential_hint: row.credential_hint,
                weight: row.weight,
                priority: row.priority,
                rpm_limit: row.rpm_limit,
                tpm_limit: row.tpm_limit,
                health: row.health,
                last_checked_at: row.last_checked_at,
                last_error: row.last_error,
                status: row.status,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
            row.provider_active,
        )
    }))
}

fn decimal_bind(value: Option<f64>) -> Option<String> {
    value.map(|value| format!("{value:.6}"))
}

fn map_unique_conflict(error: sqlx::Error, code: &'static str, detail: &'static str) -> AppError {
    if is_unique_violation(&error) {
        AppError::conflict(code, detail, None)
    } else {
        error.into()
    }
}

fn map_fk_conflict(error: sqlx::Error, code: &'static str, detail: &'static str) -> AppError {
    if is_foreign_key_violation(&error) {
        AppError::conflict(code, detail, None)
    } else {
        error.into()
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}

fn is_foreign_key_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.code().as_deref() == Some("23503"))
}
