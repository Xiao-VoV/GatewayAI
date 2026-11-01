//! 数据面鉴权：调用 access 共享入口 `authenticate_headers`，并把 `AuthError` 映射为
//! problem+json（proxy 技术设计 §4.2）。不复制 access 的 401/403 判定逻辑。

use axum::http::HeaderMap;

use crate::modules::access::model::AuthContext;
use crate::modules::access::service::{AuthError, authenticate_headers, is_model_allowed};
use crate::platform::error::AppError;
use crate::platform::state::AppState;

pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthContext, AppError> {
    authenticate_headers(state, headers)
        .await
        .map_err(map_auth_error)
}

pub fn model_allowed(context: &AuthContext, model: &str) -> bool {
    is_model_allowed(context, model)
}

/// access `AuthError` → 数据面 problem+json（固定文案，不泄露设备/Key 存在性）。
pub fn map_auth_error(error: AuthError) -> AppError {
    crate::modules::access::service::auth_error_to_app_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_missing_and_invalid_credentials_to_expected_codes() {
        let cases = [
            (AuthError::MissingApiKey, "missing_api_key"),
            (AuthError::MissingDeviceToken, "missing_device_token"),
            (AuthError::InvalidHeader, "invalid_api_key"),
            (AuthError::InvalidApiKey, "invalid_api_key"),
            (AuthError::InvalidDeviceToken, "invalid_device_token"),
            (AuthError::KeyRevoked, "key_revoked"),
            (AuthError::KeyExpired, "key_expired"),
            (AuthError::KeyDeviceMismatch, "key_device_mismatch"),
            (AuthError::DeviceDisabled, "device_disabled"),
            (AuthError::UserDisabled, "user_disabled"),
            (AuthError::Database, "database_unavailable"),
        ];
        for (error, expected) in cases {
            let mapped = map_auth_error(error);
            let code = match &mapped {
                AppError::Unauthorized { code, .. } => *code,
                AppError::Forbidden { code, .. } => *code,
                AppError::DatabaseUnavailable => "database_unavailable",
                other => panic!("unexpected mapping: {other:?}"),
            };
            assert_eq!(code, expected);
        }
    }
}
