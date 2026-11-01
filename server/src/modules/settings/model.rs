//! settings DTO：运行期设置与 PATCH 请求。
//!
//! 设计见 `设计文档（暂未开源）` §2：
//! DB `app_settings` 单例为运行期权威；GET 返回 `Settings`，PATCH 支持部分字段。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 默认值（与迁移 0012 一致；DB 不可用/缺行时兜底）。
pub const DEFAULT_REQUEST_LOG_RETENTION_DAYS: i32 = 30;
pub const DEFAULT_AUDIT_RETENTION_DAYS: i32 = 180;
pub const DEFAULT_UPSTREAM_TIMEOUT_SECS: i32 = 60;

/// 范围（服务端校验，与 DB CHECK 一致）。
pub const MIN_REQUEST_LOG_RETENTION_DAYS: i32 = 1;
pub const MAX_REQUEST_LOG_RETENTION_DAYS: i32 = 365;
pub const MIN_AUDIT_RETENTION_DAYS: i32 = 180;
pub const MAX_AUDIT_RETENTION_DAYS: i32 = 3_650;
pub const MIN_UPSTREAM_TIMEOUT_SECS: i32 = 5;
pub const MAX_UPSTREAM_TIMEOUT_SECS: i32 = 600;

/// 平台设置（GET/PATCH 响应；DB 单例行）。
#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    pub default_provider_id: Option<Uuid>,
    pub request_log_retention_days: i32,
    pub audit_retention_days: i32,
    pub upstream_timeout_secs: i32,
    pub updated_at: DateTime<Utc>,
}

impl Settings {
    /// 进程内默认值（DB 不可用/缺行兜底；不读 env）。
    pub fn defaults() -> Self {
        Self {
            default_provider_id: None,
            request_log_retention_days: DEFAULT_REQUEST_LOG_RETENTION_DAYS,
            audit_retention_days: DEFAULT_AUDIT_RETENTION_DAYS,
            upstream_timeout_secs: DEFAULT_UPSTREAM_TIMEOUT_SECS,
            updated_at: Utc::now(),
        }
    }
}

/// `PATCH /api/admin/settings` 请求：至少一个字段；`default_provider_id: null` = 清除。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSettingsRequest {
    /// 三态：字段缺省=不变；`null`=清除；UUID=设置（需 provider 存在）。
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub default_provider_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub request_log_retention_days: Option<i32>,
    #[serde(default)]
    pub audit_retention_days: Option<i32>,
    #[serde(default)]
    pub upstream_timeout_secs: Option<i32>,
}

impl PatchSettingsRequest {
    pub fn is_empty(&self) -> bool {
        self.default_provider_id.is_none()
            && self.request_log_retention_days.is_none()
            && self.audit_retention_days.is_none()
            && self.upstream_timeout_secs.is_none()
    }
}

/// `Option<Option<T>>` 的 double-option 反序列化：区分字段缺省（外层 None）
/// 与显式 `null`（`Some(None)`）。
fn deserialize_present_option<'de, D>(deserializer: D) -> Result<Option<Option<Uuid>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Uuid>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_request_distinguishes_absent_null_and_value() {
        let empty: PatchSettingsRequest = serde_json::from_str("{}").expect("empty patch");
        assert!(empty.is_empty());
        assert_eq!(empty.default_provider_id, None);

        let clear: PatchSettingsRequest =
            serde_json::from_str(r#"{"default_provider_id": null}"#).expect("clear patch");
        assert!(!clear.is_empty());
        assert_eq!(clear.default_provider_id, Some(None));

        let id = Uuid::new_v4();
        let set: PatchSettingsRequest =
            serde_json::from_str(&format!(r#"{{"default_provider_id": "{id}"}}"#))
                .expect("set patch");
        assert_eq!(set.default_provider_id, Some(Some(id)));

        let values: PatchSettingsRequest = serde_json::from_str(
            r#"{"request_log_retention_days": 7, "audit_retention_days": 200, "upstream_timeout_secs": 30}"#,
        )
        .expect("values patch");
        assert_eq!(values.request_log_retention_days, Some(7));
        assert_eq!(values.audit_retention_days, Some(200));
        assert_eq!(values.upstream_timeout_secs, Some(30));
        assert!(!values.is_empty());

        // 未知字段拒绝（deny_unknown_fields）。
        assert!(serde_json::from_str::<PatchSettingsRequest>(r#"{"bogus": 1}"#).is_err());
    }

    #[test]
    fn settings_defaults_match_migration() {
        let defaults = Settings::defaults();
        assert_eq!(defaults.default_provider_id, None);
        assert_eq!(
            defaults.request_log_retention_days,
            DEFAULT_REQUEST_LOG_RETENTION_DAYS
        );
        assert_eq!(defaults.audit_retention_days, DEFAULT_AUDIT_RETENTION_DAYS);
        assert_eq!(
            defaults.upstream_timeout_secs,
            DEFAULT_UPSTREAM_TIMEOUT_SECS
        );
        assert_eq!(DEFAULT_REQUEST_LOG_RETENTION_DAYS, 30);
        assert_eq!(DEFAULT_AUDIT_RETENTION_DAYS, 180);
        assert_eq!(DEFAULT_UPSTREAM_TIMEOUT_SECS, 60);
    }
}
