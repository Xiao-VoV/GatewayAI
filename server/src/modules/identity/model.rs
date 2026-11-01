//! identity 领域模型与请求/响应 DTO（对齐 `接口设计.md` §2）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Cookie 名（管理台专用，不与客户端 Key 混用）。
pub const SESSION_COOKIE: &str = "gw_admin_session";

/// S1 会话固定 12h（D18）；Max-Age 与 expires_at 均使用该值。
pub const SESSION_TTL_SECS: i64 = 43_200;

#[derive(Debug, Clone, Serialize)]
pub struct AdminUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SetupStatus {
    pub required: bool,
    pub setup_token_required: bool,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user: AdminUser,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}
