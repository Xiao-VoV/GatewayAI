//! routing 领域模型与请求/响应 DTO（对齐 `接口设计.md`、`接口契约.yaml` v0.1.0）。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_PAGE_LIMIT: i64 = 200;
pub const DEFAULT_PAGE_LIMIT: i64 = 50;
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_TARGETS: usize = 20;
pub const MIN_PRIORITY: i32 = 1;
pub const MAX_PRIORITY: i32 = 1000;
pub const MIN_WEIGHT: i32 = 1;
pub const MAX_WEIGHT: i32 = 100;
pub const SNAPSHOT_TTL_SECS: u64 = 5;

#[derive(Debug, Clone, Serialize)]
pub struct TargetAdmin {
    pub id: Uuid,
    pub upstream_model_id: Uuid,
    pub upstream_model: String,
    pub canonical: String,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_kind: String,
    pub provider_status: String,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ModelAliasAdmin {
    pub id: Uuid,
    pub alias: String,
    pub enabled: bool,
    pub targets: Vec<TargetAdmin>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetInput {
    pub upstream_model_id: Uuid,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub weight: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateModelAliasRequest {
    pub alias: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    pub targets: Vec<TargetInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateModelAliasRequest {
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub targets: Option<Vec<TargetInput>>,
}

#[derive(Debug, Clone)]
pub struct TargetSpec {
    pub upstream_model_id: Uuid,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewCandidate {
    pub upstream_model_id: Uuid,
    pub upstream_model: String,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_status: String,
    pub active_account_count: i64,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelAliasPreview {
    pub alias: String,
    pub enabled: bool,
    pub candidates: Vec<PreviewCandidate>,
    pub warnings: Vec<String>,
}

/// 路由来源：legacy alias（S4）或 catalog 模型组（v0.1.x 迭代 3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    Alias,
    Catalog,
}

impl RouteSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alias => "alias",
            Self::Catalog => "catalog",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoutePlan {
    pub requested_model: String,
    pub source: RouteSource,
    /// catalog 来源的模型组 id；alias 来源为 None。
    pub model_group_id: Option<Uuid>,
    /// legacy alias id；catalog 来源为 None。
    pub alias_id: Option<Uuid>,
    /// 对外模型名（catalog 为 client_model_name，alias 为 alias 名）。
    pub alias: String,
    /// catalog 来源的版本 `cat:<n>`；alias 来源为 None。
    pub catalog_version: Option<String>,
    pub candidates: Vec<RouteCandidate>,
}

#[derive(Debug, Clone)]
pub struct RouteCandidate {
    pub upstream_model_id: Uuid,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub upstream_model: String,
    pub canonical: String,
    pub priority: i32,
    pub weight: i32,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<Decimal>,
    pub price_output: Option<Decimal>,
    pub currency: String,
    /// catalog 部署 id；alias 来源为 None。
    pub deployment_id: Option<Uuid>,
    /// catalog 部署固定账号；alias 来源为 None（proxy 用 select_account）。
    pub provider_account_id: Option<Uuid>,
    /// catalog 模型组 id；alias 来源为 None。
    pub model_group_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisibleModel {
    pub id: String,
    pub object: String,
    pub created: DateTime<Utc>,
    pub upstream_model: String,
    pub canonical: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub catalog_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    ModelNotFound,
    NoRoute,
    DatabaseUnavailable,
}
