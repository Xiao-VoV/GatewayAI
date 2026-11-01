//! identity 的 SQLx 数据访问（表归属：`admin_users` / `admin_sessions`）。

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::platform::error::AppError;

/// 固定 64 位 advisory lock key（见 `数据库设计.md` §4.1）。
const IDENTITY_SETUP_LOCK_KEY: i64 = 0x1D3E_1D01;

#[derive(Debug, sqlx::FromRow)]
pub struct AdminRow {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub display_name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SessionAdminRow {
    pub session_id: Uuid,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

pub struct NewAdmin {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub display_name: String,
}

pub struct NewSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub csrf_token: String,
    pub user_agent: String,
    pub ip_address: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug)]
pub enum SetupOutcome {
    Created(AdminRow),
    AlreadyCompleted,
    Conflict,
}

pub async fn get_admin_by_id(pool: &PgPool, id: Uuid) -> Result<Option<AdminRow>, AppError> {
    let row = sqlx::query_as::<_, AdminRow>(
        "SELECT id, email, password_hash, display_name, status, created_at \
         FROM admin_users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 修改密码并吊销除当前会话外的全部管理员 Session（US-003）。
pub async fn change_password_and_revoke_sessions(
    pool: &PgPool,
    admin_id: Uuid,
    password_hash: &str,
    keep_token_hash: &str,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE admin_users SET password_hash = $1, password_changed_at = now(), updated_at = now() \
         WHERE id = $2",
    )
    .bind(password_hash)
    .bind(admin_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1 AND token_hash <> $2")
        .bind(admin_id)
        .bind(keep_token_hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn count_admins(pool: &PgPool) -> Result<i64, AppError> {
    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM admin_users")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

pub async fn find_admin_by_email(pool: &PgPool, email: &str) -> Result<Option<AdminRow>, AppError> {
    let row = sqlx::query_as::<_, AdminRow>(
        "SELECT id, email, password_hash, display_name, status, created_at \
         FROM admin_users WHERE email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 初始化：事务 + advisory lock + `admin_users` 0 行检查；唯一键冲突映射 409。
pub async fn setup_admin(pool: &PgPool, admin: NewAdmin) -> Result<SetupOutcome, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(IDENTITY_SETUP_LOCK_KEY)
        .execute(&mut *tx)
        .await?;

    let exists = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM admin_users)")
        .fetch_one(&mut *tx)
        .await?;
    if exists {
        tx.rollback().await.ok();
        return Ok(SetupOutcome::AlreadyCompleted);
    }

    let inserted = sqlx::query_as::<_, AdminRow>(
        "INSERT INTO admin_users (id, email, password_hash, display_name) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, email, password_hash, display_name, status, created_at",
    )
    .bind(admin.id)
    .bind(&admin.email)
    .bind(&admin.password_hash)
    .bind(&admin.display_name)
    .fetch_one(&mut *tx)
    .await;

    match inserted {
        Ok(row) => {
            tx.commit().await?;
            Ok(SetupOutcome::Created(row))
        }
        Err(error) if is_unique_violation(&error) => {
            tx.rollback().await.ok();
            Ok(SetupOutcome::Conflict)
        }
        Err(error) => Err(error.into()),
    }
}

/// 登录成功：同一事务内清理过期 Session、插入新 Session、更新 last_login_at。
pub async fn create_session(pool: &PgPool, session: &NewSession) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM admin_sessions WHERE expires_at < now()")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO admin_sessions \
         (id, user_id, token_hash, csrf_token, user_agent, ip_address, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(session.id)
    .bind(session.user_id)
    .bind(&session.token_hash)
    .bind(&session.csrf_token)
    .bind(&session.user_agent)
    .bind(&session.ip_address)
    .bind(session.expires_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE admin_users SET last_login_at = now(), updated_at = now() WHERE id = $1")
        .bind(session.user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// 主查询：token_hash + 未过期 + 管理员 active。
pub async fn find_session_user(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<SessionAdminRow>, AppError> {
    let row = sqlx::query_as::<_, SessionAdminRow>(
        "SELECT s.id AS session_id, s.csrf_token, s.expires_at, \
                u.id AS user_id, u.email, u.display_name, u.status, u.created_at \
         FROM admin_sessions s \
         JOIN admin_users u ON u.id = s.user_id \
         WHERE s.token_hash = $1 AND s.expires_at > now() AND u.status = 'active'",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn touch_session(pool: &PgPool, session_id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE admin_sessions SET last_seen_at = now() WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_session(pool: &PgPool, token_hash: &str) -> Result<u64, AppError> {
    let result = sqlx::query("DELETE FROM admin_sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error) if database_error.code().as_deref() == Some("23505")
    )
}
