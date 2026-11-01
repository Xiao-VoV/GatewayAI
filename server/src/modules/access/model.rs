//! access 领域模型与请求/响应 DTO（对齐 `接口设计.md` §2、`接口契约.yaml` v0.2.0）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

pub const MAX_PAGE_LIMIT: i64 = 200;
pub const DEFAULT_PAGE_LIMIT: i64 = 50;
pub const MAX_BODY_BYTES: usize = 64 * 1024;

pub const PAIRING_CODE_TTL_SECS: i64 = 600;
pub const PAIRING_CODE_MIN_TTL_SECS: i64 = 60;
pub const PAIRING_CODE_MAX_TTL_SECS: i64 = 1800;
pub const DEVICE_TOKEN_TTL_SECS: i64 = 900;

pub const MAX_ALLOWED_MODELS: usize = 100;
pub const MAX_MODEL_NAME_CHARS: usize = 100;
pub const MAX_DISPLAY_NAME_CHARS: usize = 64;
pub const MAX_DEVICE_NAME_CHARS: usize = 64;
pub const MAX_PLATFORM_CHARS: usize = 32;
pub const MAX_CLIENT_VERSION_CHARS: usize = 32;
pub const KEY_PREFIX_CHARS: usize = 12;
pub const DEVICE_ONLINE_WINDOW_SECS: i64 = 300;

/// 用户组名称/描述/成员上限（与 0013 迁移 CHECK 及用户组实现设计 §2 一致）。
pub const MAX_USER_GROUP_NAME_CHARS: usize = 64;
pub const MAX_USER_GROUP_DESCRIPTION_CHARS: usize = 200;
pub const MAX_USER_GROUP_MEMBERS: usize = 500;

pub const PAIRING_MAX_FAILURES: usize = 10;
pub const PAIRING_WINDOW_SECS: u64 = 600;
pub const TOKEN_DEVICE_MAX_FAILURES: usize = 30;
pub const TOKEN_IP_MAX_FAILURES: usize = 100;

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingCode {
    pub id: Uuid,
    pub user: User,
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PairingCodeWithCode {
    #[serde(flatten)]
    pub meta: PairingCode,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct CreatePairingCodeResponse {
    pub pairing_code: PairingCodeWithCode,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceKeySummary {
    pub id: Uuid,
    pub key_prefix: String,
    pub status: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceAdmin {
    pub id: Uuid,
    pub user: User,
    pub name: String,
    pub platform: String,
    pub client_version: String,
    pub status: String,
    pub online: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_ip: String,
    pub key: Option<DeviceKeySummary>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DeviceDetail {
    #[serde(flatten)]
    pub device: DeviceAdmin,
    pub active_token_count: i64,
    pub last_token_expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyDevice {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyAdmin {
    pub id: Uuid,
    pub key_prefix: String,
    pub status: String,
    pub user: User,
    pub device: KeyDevice,
    pub allowed_models: Vec<String>,
    /// Key 级每分钟请求上限；None=不限（S3.1）。
    pub rpm_limit: Option<i32>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ResetKeyResponse {
    pub api_key: String,
    pub key: KeyAdmin,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientDeviceSummary {
    pub id: Uuid,
    pub name: String,
    pub platform: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientDeviceInfo {
    pub id: Uuid,
    pub name: String,
    pub platform: String,
    pub client_version: String,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ClientKeySummary {
    pub id: Uuid,
    pub key_prefix: String,
    pub status: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub allowed_models: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ClientPairResponse {
    pub device: ClientDeviceSummary,
    pub user: User,
    pub device_secret: String,
    pub api_key: String,
    pub api_key_prefix: String,
    pub token: String,
    pub token_type: String,
    pub token_expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub token: String,
    pub token_type: String,
    pub expires_at: DateTime<Utc>,
    pub device_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct ClientDeviceResponse {
    pub device: ClientDeviceInfo,
    pub user: User,
    pub key: ClientKeySummary,
}

/// 管理台用户组列表项（`GET /api/admin/user-groups`）。
#[derive(Debug, Clone, Serialize)]
pub struct UserGroupSummary {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub member_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 管理台用户组详情（create/get/patch 返回；成员为完整员工摘要）。
#[derive(Debug, Serialize)]
pub struct UserGroupDetail {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub members: Vec<User>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePairingCodeRequest {
    pub user_email: String,
    #[serde(default)]
    pub user_display_name: Option<String>,
    #[serde(default)]
    pub expires_in_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchUserRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchDeviceRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchKeyRequest {
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_patch_expiry")]
    pub expires_at: Option<Option<DateTime<Utc>>>,
    /// 缺省=不改；null=清除（不限）；1..=1_000_000=设置。
    #[serde(default, deserialize_with = "deserialize_patch_rpm")]
    pub rpm_limit: Option<Option<i32>>,
}

fn deserialize_patch_rpm<'de, D>(deserializer: D) -> Result<Option<Option<i32>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<i32>::deserialize(deserializer)?;
    Ok(Some(raw))
}

fn deserialize_patch_expiry<'de, D>(
    deserializer: D,
) -> Result<Option<Option<DateTime<Utc>>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    match raw {
        None => Ok(Some(None)),
        Some(text) => DateTime::parse_from_rfc3339(text.trim())
            .map(|value| Some(Some(value.with_timezone(&Utc))))
            .map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateUserGroupRequest {
    /// trim 后 1–64 字符；大小写不敏感唯一。
    pub name: String,
    /// trim 后 ≤200 字符；缺省=空。
    #[serde(default)]
    pub description: Option<String>,
    /// 去重后 ≤500；用户必须存在。缺省=空成员。
    #[serde(default)]
    pub member_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateUserGroupRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// 三态：字段缺省=不变；`null`=清空；字符串=替换（trim 后 ≤200）。
    #[serde(default, deserialize_with = "deserialize_present_option_string")]
    pub description: Option<Option<String>>,
    /// 出现即全量替换（`[]`=清空成员；`null`/缺省=不变）。
    #[serde(default)]
    pub member_ids: Option<Vec<Uuid>>,
}

impl UpdateUserGroupRequest {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.description.is_none() && self.member_ids.is_none()
    }
}

/// `Option<Option<String>>` 反序列化：区分字段缺省（外层 None）与显式 `null`（`Some(None)`）。
fn deserialize_present_option_string<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub client_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientPairRequest {
    pub code: String,
    #[serde(default)]
    pub device: Option<DeviceInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    pub device_id: Uuid,
    pub device_secret: String,
}

/// 列表查询参数（不同端点只使用其中一部分，未知字段拒绝）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// 用户组列表查询参数：`q` 按名称/描述模糊搜索（trim；空=不过滤）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserGroupQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// 供 S4 proxy 使用的鉴权上下文（S3 内部接口）。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub key_id: Uuid,
    pub allowed_models: Vec<String>,
    /// VK 明文前缀（日志用；绝不返回明文 Key）。
    pub key_prefix: Option<String>,
    /// Key 级 RPM 上限（proxy 限流用；None=不限；不返回客户端配置面）。
    pub key_rpm_limit: Option<i32>,
    /// 用户所属用户组（catalog 发布授权用；客户端/代理响应不返回）。
    pub user_group_ids: Vec<Uuid>,
}

#[cfg(test)]
mod user_group_dto_tests {
    use super::*;

    #[test]
    fn create_request_parses_and_rejects_unknown_fields() {
        let id = Uuid::new_v4();
        let request: CreateUserGroupRequest = serde_json::from_str(&format!(
            r#"{{"name":"  Engineering  ","description":"core","member_ids":["{id}","{id}"]}}"#
        ))
        .expect("create request");
        assert_eq!(request.name, "  Engineering  ");
        assert_eq!(request.description.as_deref(), Some("core"));
        assert_eq!(request.member_ids.as_deref(), Some(&[id, id][..]));

        let minimal: CreateUserGroupRequest =
            serde_json::from_str(r#"{"name":"Product"}"#).expect("minimal create");
        assert_eq!(minimal.description, None);
        assert_eq!(minimal.member_ids, None);

        assert!(
            serde_json::from_str::<CreateUserGroupRequest>(r#"{"name":"x","bogus":1}"#).is_err()
        );
        assert!(serde_json::from_str::<CreateUserGroupRequest>(r#"{"bogus":1}"#).is_err());
    }

    #[test]
    fn update_request_distinguishes_absent_null_and_value() {
        let empty: UpdateUserGroupRequest = serde_json::from_str("{}").expect("empty patch");
        assert!(empty.is_empty());

        let clear: UpdateUserGroupRequest =
            serde_json::from_str(r#"{"description": null}"#).expect("clear description");
        assert!(!clear.is_empty());
        assert_eq!(clear.description, Some(None));
        assert_eq!(clear.member_ids, None);

        let set: UpdateUserGroupRequest = serde_json::from_str(
            r#"{"name":"Platform","description":"platform team","member_ids":[]}"#,
        )
        .expect("set patch");
        assert_eq!(set.name.as_deref(), Some("Platform"));
        assert_eq!(set.description, Some(Some("platform team".to_string())));
        assert_eq!(set.member_ids.as_deref(), Some(&[][..]));
        assert!(!set.is_empty());

        // 未知字段拒绝。
        assert!(
            serde_json::from_str::<UpdateUserGroupRequest>(r#"{"status":"disabled"}"#).is_err()
        );
    }

    #[test]
    fn query_parses_q_limit_offset() {
        let query: UserGroupQuery =
            serde_json::from_str(r#"{"q":" eng ","limit":10,"offset":20}"#).expect("query");
        assert_eq!(query.q.as_deref(), Some(" eng "));
        assert_eq!(query.limit, Some(10));
        assert_eq!(query.offset, Some(20));
        assert!(serde_json::from_str::<UserGroupQuery>(r#"{"status":"active"}"#).is_err());
    }

    #[test]
    fn summary_and_detail_serialize_expected_shape() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let summary = UserGroupSummary {
            id,
            name: "Engineering".to_string(),
            description: String::new(),
            member_count: 2,
            created_at: now,
            updated_at: now,
        };
        let value = serde_json::to_value(&summary).expect("summary json");
        for field in [
            "id",
            "name",
            "description",
            "member_count",
            "created_at",
            "updated_at",
        ] {
            assert!(value.get(field).is_some(), "summary 缺 {field}");
        }
        assert!(value.get("members").is_none());

        let user_id = Uuid::new_v4();
        let detail = UserGroupDetail {
            id,
            name: "Engineering".to_string(),
            description: "core".to_string(),
            members: vec![User {
                id: user_id,
                email: "alice@corp.example.com".to_string(),
                display_name: "Alice".to_string(),
                status: "active".to_string(),
            }],
            created_at: now,
            updated_at: now,
        };
        let value = serde_json::to_value(&detail).expect("detail json");
        assert!(value.get("member_count").is_none());
        let members = value
            .get("members")
            .and_then(|members| members.as_array())
            .expect("members array");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0]["id"], serde_json::json!(user_id));
        assert_eq!(
            members[0]["email"],
            serde_json::json!("alice@corp.example.com")
        );
        // 成员不得泄露凭证字段。
        for forbidden in ["api_key", "token", "device_secret", "key_hash"] {
            assert!(members[0].get(forbidden).is_none());
        }
    }
}
