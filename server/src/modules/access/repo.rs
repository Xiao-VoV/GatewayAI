//! access 数据访问（表归属：`users` / `devices` / `pairing_codes` / `device_tokens` /
//! `virtual_keys` / `user_groups` / `user_group_members`）。

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::platform::error::AppError;

#[derive(Debug, sqlx::FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub status: String,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct PairingCodeRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub user_email: String,
    pub user_display_name: String,
    pub user_status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct PairingCodeConsumeRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub user_email: String,
    pub user_display_name: String,
    pub user_status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct DeviceAdminRow {
    pub id: Uuid,
    pub name: String,
    pub platform: String,
    pub client_version: String,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_ip: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub user_id: Uuid,
    pub user_email: String,
    pub user_display_name: String,
    pub user_status: String,
    pub key_id: Option<Uuid>,
    pub key_prefix: Option<String>,
    pub key_status: Option<String>,
    pub key_expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct KeyAdminRow {
    pub id: Uuid,
    pub key_prefix: String,
    pub status: String,
    pub allowed_models: Vec<String>,
    pub rpm_limit: Option<i32>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub user_id: Uuid,
    pub user_email: String,
    pub user_display_name: String,
    pub user_status: String,
    pub device_id: Uuid,
    pub device_name: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct DeviceSecretRow {
    pub id: Uuid,
    #[allow(dead_code)]
    pub user_id: Uuid,
    pub status: String,
    pub user_status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct TokenValidateRow {
    pub token_id: Uuid,
    pub token_expires_at: DateTime<Utc>,
    pub device_id: Uuid,
    pub device_status: String,
    pub device_name: String,
    pub device_platform: String,
    pub device_client_version: String,
    #[allow(dead_code)]
    pub device_created_at: DateTime<Utc>,
    pub device_last_seen_at: Option<DateTime<Utc>>,
    pub user_id: Uuid,
    pub user_email: String,
    pub user_display_name: String,
    pub user_status: String,
    pub key_id: Uuid,
    pub key_prefix: String,
    pub key_status: String,
    pub key_expires_at: Option<DateTime<Utc>>,
    pub key_allowed_models: Vec<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct KeyAuthRow {
    pub key_id: Uuid,
    pub key_prefix: String,
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub key_status: String,
    pub key_expires_at: Option<DateTime<Utc>>,
    pub key_allowed_models: Vec<String>,
    pub key_rpm_limit: Option<i32>,
    pub device_status: String,
    pub user_status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct UserGroupRow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct UserGroupSummaryRow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub member_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const DEVICE_ADMIN_SELECT: &str = "SELECT d.id, d.name, d.platform, d.client_version, d.status, \
    d.last_seen_at, d.last_ip, d.created_at, d.updated_at, \
    u.id AS user_id, u.email AS user_email, u.display_name AS user_display_name, u.status AS user_status, \
    k.id AS key_id, k.key_prefix, k.status AS key_status, k.expires_at AS key_expires_at \
    FROM devices d JOIN users u ON u.id = d.user_id \
    LEFT JOIN virtual_keys k ON k.device_id = d.id";

const KEY_ADMIN_SELECT: &str = "SELECT k.id, k.key_prefix, k.status, k.allowed_models, k.rpm_limit, \
    k.expires_at, k.last_used_at, k.revoked_at, k.created_at, k.updated_at, \
    u.id AS user_id, u.email AS user_email, u.display_name AS user_display_name, u.status AS user_status, \
    d.id AS device_id, d.name AS device_name \
    FROM virtual_keys k JOIN users u ON u.id = k.user_id JOIN devices d ON d.id = k.device_id";

// ---------- users ----------

pub async fn get_user(pool: &PgPool, id: Uuid) -> Result<Option<UserRow>, AppError> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, email, display_name, status, created_at FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_users(
    pool: &PgPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<UserRow>, i64), AppError> {
    let rows = sqlx::query_as::<_, UserRow>(
        "SELECT id, email, display_name, status, created_at FROM users \
         WHERE ($1::text IS NULL OR status = $1) \
         ORDER BY created_at DESC, id DESC LIMIT $2 OFFSET $3",
    )
    .bind(status)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM users WHERE ($1::text IS NULL OR status = $1)",
    )
    .bind(status)
    .fetch_one(pool)
    .await?;
    Ok((rows, total))
}

pub async fn update_user_tx(
    conn: &mut PgConnection,
    id: Uuid,
    display_name: &str,
    status: &str,
) -> Result<Option<UserRow>, AppError> {
    let row = sqlx::query_as::<_, UserRow>(
        "UPDATE users SET display_name = $2, status = $3, updated_at = now() WHERE id = $1 \
         RETURNING id, email, display_name, status, created_at",
    )
    .bind(id)
    .bind(display_name)
    .bind(status)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

// ---------- pairing codes ----------

pub async fn invalidate_active_codes_tx(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<u64, AppError> {
    let result = sqlx::query(
        "UPDATE pairing_codes SET invalidated_at = now() \
         WHERE user_id = $1 AND used_at IS NULL AND invalidated_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected())
}

pub async fn insert_pairing_code(
    conn: &mut PgConnection,
    id: Uuid,
    code_hash: &str,
    user_id: Uuid,
    created_by: Uuid,
    expires_at: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO pairing_codes (id, code_hash, user_id, created_by, expires_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(code_hash)
    .bind(user_id)
    .bind(created_by)
    .bind(expires_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn list_pairing_codes(
    pool: &PgPool,
    status: Option<&str>,
    user_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<PairingCodeRow>, i64), AppError> {
    let derived = "CASE WHEN pc.used_at IS NOT NULL THEN 'used' \
        WHEN pc.invalidated_at IS NOT NULL THEN 'invalidated' \
        WHEN pc.expires_at <= now() THEN 'expired' ELSE 'active' END";
    let sql = format!(
        "SELECT pc.id, pc.user_id, pc.expires_at, pc.used_at, pc.invalidated_at, pc.created_at, \
         u.email AS user_email, u.display_name AS user_display_name, u.status AS user_status \
         FROM pairing_codes pc JOIN users u ON u.id = pc.user_id \
         WHERE ($1::text IS NULL OR {derived} = $1) AND ($2::uuid IS NULL OR pc.user_id = $2) \
         ORDER BY pc.created_at DESC, pc.id DESC LIMIT $3 OFFSET $4"
    );
    let rows = sqlx::query_as::<_, PairingCodeRow>(&sql)
        .bind(status)
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    let count_sql = format!(
        "SELECT count(*) FROM pairing_codes pc \
         WHERE ($1::text IS NULL OR {derived} = $1) AND ($2::uuid IS NULL OR pc.user_id = $2)"
    );
    let total = sqlx::query_scalar::<_, i64>(&count_sql)
        .bind(status)
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok((rows, total))
}

pub async fn pairing_code_exists(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pairing_codes WHERE id = $1)")
            .bind(id)
            .fetch_one(pool)
            .await?;
    Ok(exists)
}

pub async fn invalidate_pairing_code(pool: &PgPool, id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE pairing_codes SET invalidated_at = now() \
         WHERE id = $1 AND used_at IS NULL AND invalidated_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn consume_pairing_code(
    conn: &mut PgConnection,
    code_hash: &str,
) -> Result<Option<PairingCodeConsumeRow>, AppError> {
    let row = sqlx::query_as::<_, PairingCodeConsumeRow>(
        "SELECT pc.id, pc.user_id, pc.expires_at, pc.used_at, pc.invalidated_at, \
         u.email AS user_email, u.display_name AS user_display_name, u.status AS user_status \
         FROM pairing_codes pc JOIN users u ON u.id = pc.user_id \
         WHERE pc.code_hash = $1 FOR UPDATE",
    )
    .bind(code_hash)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

pub async fn mark_pairing_code_used(conn: &mut PgConnection, id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE pairing_codes SET used_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn find_or_create_user(
    conn: &mut PgConnection,
    email: &str,
    display_name: &str,
) -> Result<UserRow, AppError> {
    let existing = sqlx::query_as::<_, UserRow>(
        "SELECT id, email, display_name, status, created_at FROM users WHERE email = $1 FOR UPDATE",
    )
    .bind(email)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(mut user) = existing {
        if !display_name.is_empty() && user.display_name != display_name {
            sqlx::query("UPDATE users SET display_name = $2, updated_at = now() WHERE id = $1")
                .bind(user.id)
                .bind(display_name)
                .execute(&mut *conn)
                .await?;
            user.display_name = display_name.to_string();
        }
        return Ok(user);
    }
    let id = Uuid::new_v4();
    let user = sqlx::query_as::<_, UserRow>(
        "INSERT INTO users (id, email, display_name) VALUES ($1, $2, $3) \
         RETURNING id, email, display_name, status, created_at",
    )
    .bind(id)
    .bind(email)
    .bind(display_name)
    .fetch_one(&mut *conn)
    .await?;
    Ok(user)
}

// ---------- devices ----------

pub async fn list_devices(
    pool: &PgPool,
    status: Option<&str>,
    user_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<DeviceAdminRow>, i64), AppError> {
    let sql = format!(
        "{DEVICE_ADMIN_SELECT} \
         WHERE ($1::text IS NULL OR d.status = $1) AND ($2::uuid IS NULL OR d.user_id = $2) \
         ORDER BY d.created_at DESC, d.id DESC LIMIT $3 OFFSET $4"
    );
    let rows = sqlx::query_as::<_, DeviceAdminRow>(&sql)
        .bind(status)
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM devices d WHERE ($1::text IS NULL OR d.status = $1) \
         AND ($2::uuid IS NULL OR d.user_id = $2)",
    )
    .bind(status)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok((rows, total))
}

pub async fn get_device_admin(pool: &PgPool, id: Uuid) -> Result<Option<DeviceAdminRow>, AppError> {
    let sql = format!("{DEVICE_ADMIN_SELECT} WHERE d.id = $1");
    let row = sqlx::query_as::<_, DeviceAdminRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn device_token_stats(
    pool: &PgPool,
    device_id: Uuid,
) -> Result<(i64, Option<DateTime<Utc>>), AppError> {
    let row = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>)>(
        "SELECT count(*), max(expires_at) FROM device_tokens \
         WHERE device_id = $1 AND expires_at > now()",
    )
    .bind(device_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn insert_device(
    conn: &mut PgConnection,
    id: Uuid,
    user_id: Uuid,
    name: &str,
    platform: &str,
    client_version: &str,
    secret_hash: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO devices (id, user_id, name, platform, client_version, device_secret_hash, status) \
         VALUES ($1, $2, $3, $4, $5, $6, 'active')",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(platform)
    .bind(client_version)
    .bind(secret_hash)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn update_device_tx(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
    status: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE devices SET name = $2, status = $3, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(name)
        .bind(status)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn find_device_for_token(
    pool: &PgPool,
    device_id: Uuid,
    secret_hash: &str,
) -> Result<Option<DeviceSecretRow>, AppError> {
    let row = sqlx::query_as::<_, DeviceSecretRow>(
        "SELECT d.id, d.user_id, d.status, u.status AS user_status \
         FROM devices d JOIN users u ON u.id = d.user_id \
         WHERE d.id = $1 AND d.device_secret_hash = $2",
    )
    .bind(device_id)
    .bind(secret_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn set_device_status(
    conn: &mut PgConnection,
    id: Uuid,
    status: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE devices SET status = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn touch_device(pool: &PgPool, id: Uuid, ip: &str) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE devices SET last_seen_at = now(), last_ip = $2 \
         WHERE id = $1 AND (last_seen_at IS NULL OR last_seen_at < now() - interval '60 seconds')",
    )
    .bind(id)
    .bind(ip)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- device tokens ----------

pub async fn insert_device_token(
    conn: &mut PgConnection,
    id: Uuid,
    device_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO device_tokens (id, device_id, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(device_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn delete_device_tokens_tx(
    conn: &mut PgConnection,
    device_id: Uuid,
) -> Result<u64, AppError> {
    let result = sqlx::query("DELETE FROM device_tokens WHERE device_id = $1")
        .bind(device_id)
        .execute(&mut *conn)
        .await?;
    Ok(result.rows_affected())
}

pub async fn delete_user_tokens_tx(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<u64, AppError> {
    let result = sqlx::query(
        "DELETE FROM device_tokens USING devices \
         WHERE device_tokens.device_id = devices.id AND devices.user_id = $1",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected())
}

pub async fn delete_expired_device_tokens(pool: &PgPool, device_id: Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM device_tokens WHERE device_id = $1 AND expires_at <= now()")
        .bind(device_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn validate_device_token(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<TokenValidateRow>, AppError> {
    let row = sqlx::query_as::<_, TokenValidateRow>(
        "SELECT t.id AS token_id, t.expires_at AS token_expires_at, \
         d.id AS device_id, d.status AS device_status, d.name AS device_name, \
         d.platform AS device_platform, d.client_version AS device_client_version, \
         d.created_at AS device_created_at, d.last_seen_at AS device_last_seen_at, \
         u.id AS user_id, u.email AS user_email, u.display_name AS user_display_name, \
         u.status AS user_status, \
         k.id AS key_id, k.key_prefix, k.status AS key_status, k.expires_at AS key_expires_at, \
         k.allowed_models AS key_allowed_models \
         FROM device_tokens t JOIN devices d ON d.id = t.device_id \
         JOIN users u ON u.id = d.user_id JOIN virtual_keys k ON k.device_id = d.id \
         WHERE t.token_hash = $1",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn touch_token(pool: &PgPool, token_id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE device_tokens SET last_used_at = now() \
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '60 seconds')",
    )
    .bind(token_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- virtual keys ----------

pub async fn list_keys(
    pool: &PgPool,
    status: Option<&str>,
    user_id: Option<Uuid>,
    device_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<KeyAdminRow>, i64), AppError> {
    let sql = format!(
        "{KEY_ADMIN_SELECT} \
         WHERE ($1::text IS NULL OR k.status = $1) AND ($2::uuid IS NULL OR k.user_id = $2) \
         AND ($3::uuid IS NULL OR k.device_id = $3) \
         ORDER BY k.created_at DESC, k.id DESC LIMIT $4 OFFSET $5"
    );
    let rows = sqlx::query_as::<_, KeyAdminRow>(&sql)
        .bind(status)
        .bind(user_id)
        .bind(device_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM virtual_keys k WHERE ($1::text IS NULL OR k.status = $1) \
         AND ($2::uuid IS NULL OR k.user_id = $2) AND ($3::uuid IS NULL OR k.device_id = $3)",
    )
    .bind(status)
    .bind(user_id)
    .bind(device_id)
    .fetch_one(pool)
    .await?;
    Ok((rows, total))
}

pub async fn get_key_admin(pool: &PgPool, id: Uuid) -> Result<Option<KeyAdminRow>, AppError> {
    let sql = format!("{KEY_ADMIN_SELECT} WHERE k.id = $1");
    let row = sqlx::query_as::<_, KeyAdminRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn insert_key(
    conn: &mut PgConnection,
    id: Uuid,
    device_id: Uuid,
    user_id: Uuid,
    key_prefix: &str,
    key_hash: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO virtual_keys (id, device_id, user_id, key_prefix, key_hash, status, allowed_models) \
         VALUES ($1, $2, $3, $4, $5, 'active', '{}')",
    )
    .bind(id)
    .bind(device_id)
    .bind(user_id)
    .bind(key_prefix)
    .bind(key_hash)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn update_key(
    pool: &PgPool,
    id: Uuid,
    allowed_models: &[String],
    expires_at: Option<DateTime<Utc>>,
    rpm_limit: Option<i32>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE virtual_keys SET allowed_models = $2, expires_at = $3, rpm_limit = $4, \
         updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(allowed_models)
    .bind(expires_at)
    .bind(rpm_limit)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_key_status(
    conn: &mut PgConnection,
    device_id: Uuid,
    status: &str,
) -> Result<(), AppError> {
    match status {
        "paused" => {
            sqlx::query(
                "UPDATE virtual_keys SET status = 'paused', updated_at = now() \
                 WHERE device_id = $1 AND status = 'active'",
            )
            .bind(device_id)
            .execute(&mut *conn)
            .await?;
        }
        "active" => {
            sqlx::query(
                "UPDATE virtual_keys SET status = 'active', updated_at = now() \
                 WHERE device_id = $1 AND status = 'paused'",
            )
            .bind(device_id)
            .execute(&mut *conn)
            .await?;
        }
        _ => {
            sqlx::query(
                "UPDATE virtual_keys SET status = 'revoked', revoked_at = now(), updated_at = now() \
                 WHERE device_id = $1 AND status <> 'revoked'",
            )
            .bind(device_id)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

pub async fn revoke_key(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let updated = sqlx::query(
        "UPDATE virtual_keys SET status = 'revoked', revoked_at = now(), updated_at = now() \
         WHERE id = $1 AND status <> 'revoked' RETURNING id",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    if updated.is_some() {
        return Ok(true);
    }
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM virtual_keys WHERE id = $1)")
            .bind(id)
            .fetch_one(pool)
            .await?;
    Ok(exists)
}

pub async fn reset_key(
    conn: &mut PgConnection,
    id: Uuid,
    key_prefix: &str,
    key_hash: &str,
) -> Result<bool, AppError> {
    let updated = sqlx::query(
        "UPDATE virtual_keys SET key_prefix = $2, key_hash = $3, status = 'active', \
         revoked_at = NULL, last_used_at = NULL, updated_at = now() \
         WHERE id = $1 RETURNING id",
    )
    .bind(id)
    .bind(key_prefix)
    .bind(key_hash)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(updated.is_some())
}

pub async fn find_key_for_auth(
    pool: &PgPool,
    key_hash: &str,
) -> Result<Option<KeyAuthRow>, AppError> {
    let row = sqlx::query_as::<_, KeyAuthRow>(
        "SELECT k.id AS key_id, k.key_prefix, k.user_id, k.device_id, k.status AS key_status, \
         k.expires_at AS key_expires_at, k.allowed_models AS key_allowed_models, \
         k.rpm_limit AS key_rpm_limit, d.status AS device_status, u.status AS user_status \
         FROM virtual_keys k JOIN devices d ON d.id = k.device_id JOIN users u ON u.id = k.user_id \
         WHERE k.key_hash = $1",
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn touch_key(pool: &PgPool, key_id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE virtual_keys SET last_used_at = now() \
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '60 seconds')",
    )
    .bind(key_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- user groups ----------

fn user_group_summary_where() -> &'static str {
    "($1::text IS NULL OR g.name ILIKE '%' || $1 || '%' \
        OR g.description ILIKE '%' || $1 || '%')"
}

pub async fn list_user_groups(
    pool: &PgPool,
    q: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<UserGroupSummaryRow>, i64), AppError> {
    let sql = format!(
        "SELECT g.id, g.name, g.description, \
         (SELECT count(*) FROM user_group_members m WHERE m.group_id = g.id) AS member_count, \
         g.created_at, g.updated_at FROM user_groups g \
         WHERE {} ORDER BY g.updated_at DESC, g.id DESC LIMIT $2 OFFSET $3",
        user_group_summary_where()
    );
    let rows = sqlx::query_as::<_, UserGroupSummaryRow>(&sql)
        .bind(q)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    let total = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT count(*) FROM user_groups g WHERE {}",
        user_group_summary_where()
    ))
    .bind(q)
    .fetch_one(pool)
    .await?;
    Ok((rows, total))
}

pub async fn get_user_group(pool: &PgPool, id: Uuid) -> Result<Option<UserGroupRow>, AppError> {
    Ok(sqlx::query_as::<_, UserGroupRow>(
        "SELECT id, name, description, created_at, updated_at FROM user_groups WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// PATCH 事务内锁定组行（并发成员替换串行化）。
pub async fn get_user_group_for_update(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<UserGroupRow>, AppError> {
    Ok(sqlx::query_as::<_, UserGroupRow>(
        "SELECT id, name, description, created_at, updated_at FROM user_groups \
         WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?)
}

pub async fn insert_user_group(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
    description: &str,
) -> Result<UserGroupRow, AppError> {
    let row = sqlx::query_as::<_, UserGroupRow>(
        "INSERT INTO user_groups (id, name, description) VALUES ($1, $2, $3) \
         RETURNING id, name, description, created_at, updated_at",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .fetch_one(&mut *conn)
    .await;
    row.map_err(map_user_group_write_error)
}

pub async fn update_user_group(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
    description: &str,
) -> Result<Option<UserGroupRow>, AppError> {
    let row = sqlx::query_as::<_, UserGroupRow>(
        "UPDATE user_groups SET name = $2, description = $3, updated_at = now() \
         WHERE id = $1 RETURNING id, name, description, created_at, updated_at",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .fetch_optional(&mut *conn)
    .await;
    row.map_err(map_user_group_write_error)
}

/// 物理删除组（成员按 FK CASCADE 删除）；返回是否实际删除（幂等语义由 service 处理）。
pub async fn delete_user_group(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let deleted = sqlx::query("DELETE FROM user_groups WHERE id = $1 RETURNING id")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(deleted.is_some())
}

pub async fn list_user_group_members(
    pool: &PgPool,
    group_id: Uuid,
) -> Result<Vec<UserRow>, AppError> {
    Ok(sqlx::query_as::<_, UserRow>(
        "SELECT u.id, u.email, u.display_name, u.status, u.created_at \
         FROM user_group_members m JOIN users u ON u.id = m.user_id \
         WHERE m.group_id = $1 ORDER BY lower(u.email) ASC, u.id ASC",
    )
    .bind(group_id)
    .fetch_all(pool)
    .await?)
}

/// 全量替换组成员（调用方须在持有组行 FOR UPDATE 的事务内调用）。
pub async fn replace_user_group_members(
    conn: &mut PgConnection,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM user_group_members WHERE group_id = $1")
        .bind(group_id)
        .execute(&mut *conn)
        .await?;
    if user_ids.is_empty() {
        return Ok(());
    }
    let result = sqlx::query(
        "INSERT INTO user_group_members (group_id, user_id) \
         SELECT $1, unnest($2::uuid[])",
    )
    .bind(group_id)
    .bind(user_ids)
    .execute(&mut *conn)
    .await;
    result.map_err(map_member_write_error)?;
    Ok(())
}

/// 事务内校验成员用户存在性（只查 id；disabled 用户允许入组）。
pub async fn existing_user_ids(
    conn: &mut PgConnection,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, AppError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows.into_iter().collect())
}

/// 内部接口：用户所属组 id（catalog 发布授权用）。
pub async fn user_group_ids(pool: &PgPool, user_id: Uuid) -> Result<Vec<Uuid>, AppError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT group_id FROM user_group_members WHERE user_id = $1 ORDER BY group_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// 内部接口：组是否存在（catalog 发布对象校验用）。
pub async fn user_group_exists(pool: &PgPool, group_id: Uuid) -> Result<bool, AppError> {
    Ok(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM user_groups WHERE id = $1)")
            .bind(group_id)
            .fetch_one(pool)
            .await?,
    )
}

// ---------- 错误映射 ----------

fn constraint_name(error: &sqlx::Error) -> Option<&str> {
    match error {
        sqlx::Error::Database(database_error) => database_error.constraint(),
        _ => None,
    }
}

fn map_user_group_write_error(error: sqlx::Error) -> AppError {
    match constraint_name(&error) {
        Some("user_groups_name_key") => AppError::conflict(
            "user_group_name_conflict",
            "用户组名称已存在（大小写不敏感）",
            None,
        ),
        Some(_) | None => AppError::from(error),
    }
}

fn map_member_write_error(error: sqlx::Error) -> AppError {
    match constraint_name(&error) {
        Some("user_group_members_user_id_fkey") => {
            AppError::not_found("user_not_found", "员工用户不存在")
        }
        Some(_) | None => AppError::from(error),
    }
}
