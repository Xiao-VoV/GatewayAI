//! providers 领域模型与请求/响应 DTO（对齐 `接口设计.md` §2）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const KIND_OPENAI_COMPATIBLE: &str = "openai_compatible";
pub const MAX_ACCOUNTS: usize = 50;
pub const MAX_MODELS: usize = 200;
pub const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Provider {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub description: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ProviderDetail {
    #[serde(flatten)]
    pub provider: Provider,
    pub account_count: i64,
    pub active_account_count: i64,
    pub upstream_model_count: i64,
    pub model_alias_count: i64,
    pub routable: bool,
}

#[derive(Debug, Serialize)]
pub struct ProviderPage {
    pub items: Vec<Provider>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderAccount {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub name: String,
    pub region: String,
    pub api_key_hint: String,
    pub weight: i32,
    pub priority: i32,
    pub rpm_limit: Option<i32>,
    pub tpm_limit: Option<i64>,
    pub health: String,
    pub status: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AccountList {
    pub items: Vec<ProviderAccount>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpstreamModel {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub name: String,
    pub canonical: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ModelList {
    pub items: Vec<UpstreamModel>,
}

#[derive(Debug, Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub checked_at: DateTime<Utc>,
    pub account_id: Option<Uuid>,
    pub http_status: Option<u16>,
    pub error: Option<TestError>,
}

#[derive(Debug, Serialize)]
pub struct TestError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProviderRequest {
    pub name: String,
    pub kind: String,
    pub base_url: String,
    #[serde(default)]
    pub description: String,
    pub account: CreateAccountRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAccountRequest {
    pub name: String,
    pub api_key: String,
    #[serde(default)]
    pub region: String,
    pub weight: Option<i32>,
    pub priority: Option<i32>,
    pub rpm_limit: Option<i32>,
    pub tpm_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchProviderRequest {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchAccountRequest {
    pub name: Option<String>,
    pub api_key: Option<String>,
    pub region: Option<String>,
    pub weight: Option<i32>,
    pub priority: Option<i32>,
    pub rpm_limit: Option<i32>,
    pub tpm_limit: Option<i64>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateModelRequest {
    pub name: String,
    pub canonical: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchModelRequest {
    pub name: Option<String>,
    pub canonical: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TestRequest {
    pub account_id: Option<Uuid>,
    pub model: Option<String>,
}

/// routing 快照读取的上游模型静态视图（不含凭证/账号状态）。
#[derive(Debug, Clone)]
pub struct RoutingUpstreamView {
    pub upstream_model_id: Uuid,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub upstream_model: String,
    pub canonical: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
}

/// 管理台路由预览用的实时 provider 状态/账号数。
#[derive(Debug, Clone)]
pub struct ProviderStatusView {
    pub provider_id: Uuid,
    pub status: String,
    pub active_account_count: i64,
}
