//! settings 数据访问（表归属：`app_settings` 单例行 id=1）。
//!
//! PATCH 在单事务内 `SELECT ... FOR UPDATE` → 合并 → `UPDATE ... RETURNING`，
//! 避免并发管理写覆盖丢失（单管理员场景，后写覆盖可接受）。

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::platform::error::AppError;

const SETTINGS_COLUMNS: &str = "default_provider_id, request_log_retention_days, \
    audit_retention_days, upstream_timeout_secs, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SettingsRow {
    pub default_provider_id: Option<Uuid>,
    pub request_log_retention_days: i32,
    pub audit_retention_days: i32,
    pub upstream_timeout_secs: i32,
    pub updated_at: DateTime<Utc>,
}

/// 读取单例设置（启动加载/GET）。
pub async fn get_settings(pool: &PgPool) -> Result<Option<SettingsRow>, AppError> {
    let row = sqlx::query_as::<_, SettingsRow>(&format!(
        "SELECT {SETTINGS_COLUMNS} FROM app_settings WHERE id = 1"
    ))
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 事务内锁定单例行（PATCH 读-改-写；防并发丢失更新）。
pub async fn lock_settings(conn: &mut PgConnection) -> Result<Option<SettingsRow>, AppError> {
    let row = sqlx::query_as::<_, SettingsRow>(&format!(
        "SELECT {SETTINGS_COLUMNS} FROM app_settings WHERE id = 1 FOR UPDATE"
    ))
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// 全字段 UPDATE（仅出现字段已由 service 合并进 `row`），返回最新行。
pub async fn update_settings(
    conn: &mut PgConnection,
    row: &SettingsRow,
) -> Result<SettingsRow, AppError> {
    let updated = sqlx::query_as::<_, SettingsRow>(&format!(
        "UPDATE app_settings SET \
             default_provider_id = $1, request_log_retention_days = $2, \
             audit_retention_days = $3, upstream_timeout_secs = $4, updated_at = now() \
         WHERE id = 1 RETURNING {SETTINGS_COLUMNS}"
    ))
    .bind(row.default_provider_id)
    .bind(row.request_log_retention_days)
    .bind(row.audit_retention_days)
    .bind(row.upstream_timeout_secs)
    .fetch_one(&mut *conn)
    .await?;
    Ok(updated)
}
