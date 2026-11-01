//! 管理台鉴权件：`AdminSession` 提取器 + CSRF 校验。
//!
//! 契约见 `技术设计.md` §1.5；后续管理台模块复用同一提取器。
//! 另提供 `admin_session_from_headers` 供 observability 全局审计 middleware 使用
//! （只读解析、不更新 `last_seen_at`，避免每个管理写请求额外一次 UPDATE）。

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, request::Parts},
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::platform::{error::AppError, http as platform_http, state::AppState};

use super::{model::AdminUser, repo, security};

pub struct AdminSession {
    pub token_hash: String,
    pub user: AdminUser,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
}

impl AdminSession {
    /// 复用的解析逻辑：Cookie token_hash → admin_sessions + admin_users 校验。
    ///
    /// 返回 `(session, session_id)`；`session_id` 供官方提取器更新 `last_seen_at`。
    async fn resolve(state: &AppState, headers: &HeaderMap) -> Result<(Self, Uuid), AppError> {
        let token = platform_http::cookie_value(headers, super::model::SESSION_COOKIE)
            .ok_or_else(AppError::not_authenticated)?;
        let pool = state
            .db
            .as_ref()
            .ok_or_else(AppError::database_unavailable)?;
        let token_hash = security::sha256_hex(&token);

        let row = repo::find_session_user(pool, &token_hash)
            .await?
            .ok_or_else(AppError::not_authenticated)?;
        let session_id = row.session_id;
        let session = Self {
            token_hash,
            user: AdminUser {
                id: row.user_id,
                email: row.email,
                display_name: row.display_name,
                status: row.status,
                created_at: row.created_at,
            },
            csrf_token: row.csrf_token,
            expires_at: row.expires_at,
        };
        Ok((session, session_id))
    }
}

impl FromRequestParts<AppState> for AdminSession {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let (session, session_id) = Self::resolve(state, &parts.headers).await?;

        // last_seen_at 仅展示用；失败不阻断请求（保持既有行为）。
        if let Some(pool) = state.db.as_ref()
            && let Err(error) = repo::touch_session(pool, session_id).await
        {
            tracing::warn!(error = %error, "update admin_sessions.last_seen_at failed");
        }

        Ok(session)
    }
}

/// 从原始请求头解析管理员 Session（供全局审计 middleware）。
///
/// - 无 Cookie / Session 过期 / 无效 / DB 不可用 → `None`（审计按匿名处理）；
/// - **不**更新 `last_seen_at`（审计旁路，避免写放大）；认证提取器行为不变。
pub async fn admin_session_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<AdminSession> {
    match AdminSession::resolve(state, headers).await {
        Ok((session, _)) => Some(session),
        Err(_) => None,
    }
}

/// 已登录写请求校验 `X-CSRF-Token`；缺失/错误统一 403 `csrf_invalid`。
pub fn require_csrf(session: &AdminSession, headers: &HeaderMap) -> Result<(), AppError> {
    let provided = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if provided.is_empty() || !security::constant_time_eq(provided, &session.csrf_token) {
        return Err(AppError::csrf_invalid());
    }
    Ok(())
}
