//! catalog 领域模型与 DTO。
//!
//! - S4 数据面：`ClientCatalogList`/`ClientModel`（routing legacy alias 投影的稳定形状）。
//! - v0.1.x 控制面（本迭代）：模型组 / 部署的管理台 DTO、请求校验、warnings 规则。
//!
//! 契约：`contracts/catalog.yaml`；实现设计：
//! `设计文档（暂未开源）`。
//! 隐私：客户端投影不含 canonical/价格/供应商/账号/allowed；管理台可含内部字段。

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::platform::error::AppError;

// ---------- 通用常量 ----------

pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_PAGE_LIMIT: i64 = 200;
pub const DEFAULT_PAGE_LIMIT: i64 = 50;
pub const MAX_NESTED_DEPLOYMENTS: usize = 50;
pub const MAX_CAPABILITIES: usize = 32;
pub const MAX_PROTOCOLS: usize = 8;
pub const MAX_PARAMETERS_BYTES: usize = 4 * 1024;
pub const MIN_PRIORITY: i32 = 1;
pub const MAX_PRIORITY: i32 = 1000;
pub const MIN_WEIGHT: i32 = 1;
pub const MAX_WEIGHT: i32 = 100;

pub const GROUP_STATUS_DRAFT: &str = "draft";
pub const GROUP_STATUS_ACTIVE: &str = "active";
pub const GROUP_STATUS_ARCHIVED: &str = "archived";

pub const PROTOCOL_VALUES: [&str; 4] = [
    "openai_chat",
    "anthropic_messages",
    "openai_responses",
    "gemini_generate_content",
];

// ---------- S4 数据面 DTO（保持不变，v0.1.x 只增字段） ----------

#[derive(Debug, Serialize)]
pub struct ClientCatalogList {
    pub object: &'static str,
    pub data: Vec<ClientModel>,
    pub catalog_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientModel {
    pub id: String,
    pub object: &'static str,
    /// Unix 秒。
    pub created: i64,
    pub owned_by: &'static str,
    pub display_name: String,
    pub upstream_model: Option<String>,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    /// S4 暂无协议发布信息（空数组）；v0.1.x 只增字段。
    pub protocols: Vec<String>,
}

// ---------- routing/proxy 推理接入只读视图（迭代 3） ----------

/// 路由解析需要的模型组视图（包含 draft/archived，交 routing 过滤 active）。
///
/// 组级 display/capabilities/context 字段为后续策略/透传预留；当前 routing 只消费
/// `status/client_model_name/canonical/deployments`，用 `allow(dead_code)` 标注接缝。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RoutingGroupView {
    pub group_id: Uuid,
    pub client_model_name: String,
    pub display_name: String,
    pub canonical: String,
    /// active / draft / archived；routing 只把 active 视为已接管名称。
    pub status: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub deployments: Vec<RoutingDeploymentView>,
}

/// 部署 + providers 静态富化信息（账号由 proxy 实时校验/解密，不在 catalog 选择）。
#[derive(Debug, Clone)]
pub struct RoutingDeploymentView {
    pub deployment_id: Uuid,
    pub upstream_model_id: Uuid,
    pub provider_account_id: Uuid,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub upstream_model: String,
    pub canonical: String,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<Decimal>,
    pub price_output: Option<Decimal>,
    pub currency: String,
}

// ---------- 管理台响应 DTO ----------

#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicationSummary {
    pub id: Uuid,
    pub status: String,
    pub target_count: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelGroupSummary {
    pub id: Uuid,
    pub client_model_name: String,
    pub display_name: String,
    pub canonical: String,
    pub status: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub deployment_count: i64,
    pub enabled_deployment_count: i64,
    pub publication_status: Option<String>,
    pub tool_default_count: i64,
    pub warnings: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelGroupAdmin {
    #[serde(flatten)]
    pub summary: ModelGroupSummary,
    pub description: String,
    pub protocols: Vec<String>,
    pub parameters: Value,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
    pub deployments: Vec<DeploymentAdmin>,
    /// 本迭代未实现发布；固定空数组（字段保留）。
    pub publications: Vec<PublicationSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeploymentAdmin {
    pub id: Uuid,
    pub model_group_id: Option<Uuid>,
    pub model_group_client_name: Option<String>,
    pub upstream_model_id: Uuid,
    pub upstream_model: String,
    pub canonical: String,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_kind: String,
    pub provider_status: String,
    pub provider_account_id: Uuid,
    pub account_name: String,
    pub account_region: String,
    pub account_status: String,
    pub account_health: String,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub notes: String,
    pub warnings: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutePreviewCandidate {
    pub deployment_id: Uuid,
    pub upstream_model_id: Uuid,
    pub upstream_model: String,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_status: String,
    pub provider_account_id: Uuid,
    pub account_name: String,
    pub account_status: String,
    pub account_health: String,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelGroupRoutePreview {
    pub model_group_id: Uuid,
    pub client_model_name: String,
    pub status: String,
    pub candidates: Vec<RoutePreviewCandidate>,
    pub warnings: Vec<String>,
}

// ---------- 管理台查询参数 ----------

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelGroupListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub canonical: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentListQuery {
    #[serde(default)]
    pub model_group_id: Option<Uuid>,
    #[serde(default)]
    pub upstream_model_id: Option<Uuid>,
    #[serde(default)]
    pub provider_id: Option<Uuid>,
    #[serde(default)]
    pub provider_account_id: Option<Uuid>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub ungrouped: Option<bool>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct PageParams {
    pub limit: i64,
    pub offset: i64,
}

// ---------- 管理台请求 DTO ----------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelGroupDeploymentInput {
    pub upstream_model_id: Uuid,
    pub provider_account_id: Uuid,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub weight: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelGroupCreateRequest {
    pub client_model_name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub canonical: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
    #[serde(default)]
    pub context_window: Option<i32>,
    #[serde(default)]
    pub max_output: Option<i32>,
    #[serde(default)]
    pub protocols: Option<Vec<String>>,
    #[serde(default)]
    pub parameters: Option<Value>,
    #[serde(default)]
    pub price_input: Option<f64>,
    #[serde(default)]
    pub price_output: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub deployments: Option<Vec<ModelGroupDeploymentInput>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelGroupUpdateRequest {
    #[serde(default)]
    pub client_model_name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub canonical: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub context_window: Option<Option<i32>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub max_output: Option<Option<i32>>,
    #[serde(default)]
    pub protocols: Option<Vec<String>>,
    #[serde(default)]
    pub parameters: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub price_input: Option<Option<f64>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub price_output: Option<Option<f64>>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentCreateRequest {
    pub upstream_model_id: Uuid,
    pub provider_account_id: Uuid,
    #[serde(default)]
    pub model_group_id: Option<Uuid>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub weight: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentUpdateRequest {
    /// 缺省 = 不改；null = 移出模型组；uuid = 入组/移组。
    #[serde(default, deserialize_with = "deserialize_some")]
    pub model_group_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub weight: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// `Option<Option<T>>` 的 PATCH 语义：缺省 = 不改，`null` = 显式清空。
fn deserialize_some<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

// ---------- 校验后的领域输入 ----------

#[derive(Debug, Clone)]
pub struct GroupFields {
    pub client_model_name: String,
    pub display_name: String,
    pub canonical: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub protocols: Vec<String>,
    pub parameters: Value,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
    pub status: String,
}

#[derive(Debug, Default, Clone)]
pub struct GroupPatch {
    pub client_model_name: Option<String>,
    pub display_name: Option<String>,
    pub canonical: Option<String>,
    pub description: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub context_window: Option<Option<i32>>,
    pub max_output: Option<Option<i32>>,
    pub protocols: Option<Vec<String>>,
    pub parameters: Option<Value>,
    pub price_input: Option<Option<f64>>,
    pub price_output: Option<Option<f64>>,
    pub currency: Option<String>,
    pub status: Option<String>,
}

impl GroupPatch {
    pub fn is_empty(&self) -> bool {
        self.client_model_name.is_none()
            && self.display_name.is_none()
            && self.canonical.is_none()
            && self.description.is_none()
            && self.capabilities.is_none()
            && self.context_window.is_none()
            && self.max_output.is_none()
            && self.protocols.is_none()
            && self.parameters.is_none()
            && self.price_input.is_none()
            && self.price_output.is_none()
            && self.currency.is_none()
            && self.status.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct DeploymentSpec {
    pub upstream_model_id: Uuid,
    pub provider_account_id: Uuid,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub notes: String,
}

#[derive(Debug, Default, Clone)]
pub struct DeploymentPatch {
    pub model_group_id: Option<Option<Uuid>>,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
    pub enabled: Option<bool>,
    pub notes: Option<String>,
}

impl DeploymentPatch {
    pub fn is_empty(&self) -> bool {
        self.model_group_id.is_none()
            && self.priority.is_none()
            && self.weight.is_none()
            && self.enabled.is_none()
            && self.notes.is_none()
    }
}

// ---------- 纯校验 ----------

fn collect(errors: &mut BTreeMap<String, String>, field: &str, message: impl Into<String>) {
    errors
        .entry(field.to_string())
        .or_insert_with(|| message.into());
}

fn validate_client_model_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.len() > 64 {
        return Err("客户端模型名必须为 1–64 个字符".to_string());
    }
    let bytes = name.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err("客户端模型名必须以字母或数字开头".to_string());
    }
    let valid = bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'-'));
    if !valid {
        return Err("客户端模型名仅允许字母数字与 . _ : -".to_string());
    }
    Ok(name.to_string())
}

fn validate_canonical(raw: &str) -> Result<String, String> {
    let canonical = raw.trim();
    if canonical.is_empty() || canonical.chars().count() > 100 {
        return Err("canonical 必须为 1–100 个非空白字符".to_string());
    }
    if canonical.chars().any(char::is_whitespace) {
        return Err("canonical 不能包含空白字符".to_string());
    }
    Ok(canonical.to_string())
}

fn validate_display_name(raw: Option<&str>) -> Result<String, String> {
    let value = raw.unwrap_or_default().trim();
    if value.chars().count() > 128 {
        return Err("display_name 最多 128 个字符".to_string());
    }
    Ok(value.to_string())
}

fn validate_optional_text(
    raw: Option<&str>,
    field: &str,
    max_chars: usize,
) -> Result<String, String> {
    let value = raw.unwrap_or_default().trim();
    if value.chars().count() > max_chars {
        return Err(format!("{field} 最多 {max_chars} 个字符"));
    }
    Ok(value.to_string())
}

fn validate_capabilities(raw: Option<&[String]>) -> Result<Vec<String>, String> {
    let Some(values) = raw else {
        return Ok(Vec::new());
    };
    if values.len() > MAX_CAPABILITIES {
        return Err(format!("capabilities 最多 {MAX_CAPABILITIES} 项"));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let item = value.trim();
        let valid = (1..=32).contains(&item.len())
            && item.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            && item
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !valid {
            return Err(
                "capabilities 每项需为 1–32 位小写字母/数字/下划线，且以字母开头".to_string(),
            );
        }
        if seen.insert(item.to_string()) {
            out.push(item.to_string());
        }
    }
    Ok(out)
}

fn validate_protocols(raw: Option<&[String]>) -> Result<Vec<String>, String> {
    let Some(values) = raw else {
        return Ok(Vec::new());
    };
    if values.len() > MAX_PROTOCOLS {
        return Err(format!("protocols 最多 {MAX_PROTOCOLS} 项"));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let item = value.trim();
        if !PROTOCOL_VALUES.contains(&item) {
            return Err(format!(
                "protocols 取值必须是 {} 之一",
                PROTOCOL_VALUES.join("/")
            ));
        }
        if seen.insert(item.to_string()) {
            out.push(item.to_string());
        }
    }
    Ok(out)
}

fn validate_parameters(raw: Option<Value>) -> Result<Value, String> {
    let value = raw.unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    if !value.is_object() {
        return Err("parameters 必须是 JSON 对象".to_string());
    }
    match serde_json::to_vec(&value) {
        Ok(bytes) if bytes.len() <= MAX_PARAMETERS_BYTES => Ok(value),
        Ok(_) => Err(format!(
            "parameters 序列化后不能超过 {MAX_PARAMETERS_BYTES} 字节"
        )),
        Err(_) => Err("parameters 不是合法 JSON".to_string()),
    }
}

fn validate_price(raw: Option<f64>, field: &str) -> Result<Option<f64>, String> {
    match raw {
        None => Ok(None),
        Some(value) if value.is_finite() && value >= 0.0 => Ok(Some(value)),
        Some(_) => Err(format!("{field} 必须是非负有限数字")),
    }
}

fn validate_currency(raw: Option<&str>) -> Result<String, String> {
    let currency = raw.unwrap_or("CNY").trim().to_uppercase();
    if currency == "CNY" || currency == "USD" {
        Ok(currency)
    } else {
        Err("currency 仅支持 CNY/USD".to_string())
    }
}

fn validate_status(raw: Option<&str>, allow_archived: bool) -> Result<String, String> {
    let status = raw.unwrap_or(GROUP_STATUS_DRAFT).trim();
    match status {
        GROUP_STATUS_DRAFT | GROUP_STATUS_ACTIVE => Ok(status.to_string()),
        GROUP_STATUS_ARCHIVED if allow_archived => Ok(status.to_string()),
        GROUP_STATUS_ARCHIVED => Err("创建时不能直接设置 archived".to_string()),
        _ => Err("status 仅支持 draft/active/archived".to_string()),
    }
}

fn validate_context_window(raw: Option<i32>) -> Result<Option<i32>, String> {
    match raw {
        None => Ok(None),
        Some(value) if value > 0 => Ok(Some(value)),
        Some(_) => Err("context_window 必须大于 0".to_string()),
    }
}

fn validate_max_output(raw: Option<i32>) -> Result<Option<i32>, String> {
    match raw {
        None => Ok(None),
        Some(value) if value > 0 => Ok(Some(value)),
        Some(_) => Err("max_output 必须大于 0".to_string()),
    }
}

fn validate_priority(raw: Option<i32>) -> Result<i32, String> {
    let value = raw.unwrap_or(MIN_PRIORITY);
    if (MIN_PRIORITY..=MAX_PRIORITY).contains(&value) {
        Ok(value)
    } else {
        Err(format!("priority 必须在 {MIN_PRIORITY}–{MAX_PRIORITY}"))
    }
}

fn validate_weight(raw: Option<i32>) -> Result<i32, String> {
    let value = raw.unwrap_or(MAX_WEIGHT);
    if (MIN_WEIGHT..=MAX_WEIGHT).contains(&value) {
        Ok(value)
    } else {
        Err(format!("weight 必须在 {MIN_WEIGHT}–{MAX_WEIGHT}"))
    }
}

/// 上下文/输出上限的关系校验（`max_output <= context_window`）。
pub fn check_context_pair(
    context_window: Option<i32>,
    max_output: Option<i32>,
) -> Result<(), (String, String)> {
    if let (Some(context), Some(max_output)) = (context_window, max_output)
        && max_output > context
    {
        return Err((
            "max_output".to_string(),
            "max_output 不能大于 context_window".to_string(),
        ));
    }
    Ok(())
}

// ---------- 请求校验入口 ----------

pub fn validate_page(limit: Option<i64>, offset: Option<i64>) -> Result<PageParams, AppError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err(AppError::validation_field(
            "limit",
            format!("limit 必须在 1–{MAX_PAGE_LIMIT}"),
        ));
    }
    let offset = offset.unwrap_or(0);
    if offset < 0 {
        return Err(AppError::validation_field("offset", "offset 不能为负数"));
    }
    Ok(PageParams { limit, offset })
}

pub fn validate_group_status_filter(raw: Option<&str>) -> Result<Option<&str>, AppError> {
    match raw {
        None => Ok(None),
        Some(value)
            if [
                GROUP_STATUS_DRAFT,
                GROUP_STATUS_ACTIVE,
                GROUP_STATUS_ARCHIVED,
            ]
            .contains(&value) =>
        {
            Ok(Some(value))
        }
        Some(_) => Err(AppError::validation_field(
            "status",
            "status 仅支持 draft/active/archived",
        )),
    }
}

pub fn validate_list_filters(
    canonical: Option<&str>,
    q: Option<&str>,
) -> Result<(Option<String>, Option<String>), AppError> {
    let canonical = match canonical.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.chars().count() <= 100 => Some(value.to_string()),
        Some(_) => {
            return Err(AppError::validation_field(
                "canonical",
                "canonical 最多 100 个字符",
            ));
        }
        None => None,
    };
    let q = match q.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.chars().count() <= 64 => Some(value.to_string()),
        Some(_) => {
            return Err(AppError::validation_field("q", "q 最多 64 个字符"));
        }
        None => None,
    };
    Ok((canonical, q))
}

/// 校验模型组创建请求；返回（组字段，嵌套部署）。
pub fn validate_group_create(
    request: ModelGroupCreateRequest,
) -> Result<(GroupFields, Vec<DeploymentSpec>), AppError> {
    let mut errors = BTreeMap::new();

    let client_model_name = match validate_client_model_name(&request.client_model_name) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "client_model_name", message);
            String::new()
        }
    };
    let canonical = match validate_canonical(&request.canonical) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "canonical", message);
            String::new()
        }
    };
    let display_name = match validate_display_name(request.display_name.as_deref()) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "display_name", message);
            String::new()
        }
    };
    let description =
        match validate_optional_text(request.description.as_deref(), "description", 500) {
            Ok(value) => value,
            Err(message) => {
                collect(&mut errors, "description", message);
                String::new()
            }
        };
    let capabilities = match validate_capabilities(request.capabilities.as_deref()) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "capabilities", message);
            Vec::new()
        }
    };
    let protocols = match validate_protocols(request.protocols.as_deref()) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "protocols", message);
            Vec::new()
        }
    };
    let parameters = match validate_parameters(request.parameters) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "parameters", message);
            Value::Object(serde_json::Map::new())
        }
    };
    let context_window = match validate_context_window(request.context_window) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "context_window", message);
            None
        }
    };
    let max_output = match validate_max_output(request.max_output) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "max_output", message);
            None
        }
    };
    if let Err((field, message)) = check_context_pair(context_window, max_output) {
        collect(&mut errors, &field, message);
    }
    let price_input = match validate_price(request.price_input, "price_input") {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "price_input", message);
            None
        }
    };
    let price_output = match validate_price(request.price_output, "price_output") {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "price_output", message);
            None
        }
    };
    let currency = match validate_currency(request.currency.as_deref()) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "currency", message);
            "CNY".to_string()
        }
    };
    let status = match validate_status(request.status.as_deref(), false) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "status", message);
            GROUP_STATUS_DRAFT.to_string()
        }
    };

    let deployment_inputs = request.deployments.unwrap_or_default();
    let deployments = match validate_deployment_inputs(&deployment_inputs) {
        Ok(value) => value,
        Err(deployment_errors) => {
            for (field, message) in deployment_errors {
                collect(&mut errors, "deployments", format!("{field}: {message}"));
            }
            Vec::new()
        }
    };

    if errors.is_empty() && status == GROUP_STATUS_ACTIVE && !deployments.iter().any(|d| d.enabled)
    {
        return Err(AppError::bad_request(
            "no_enabled_deployment",
            "active 模型组至少需要一个启用的部署",
        ));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

    Ok((
        GroupFields {
            client_model_name,
            display_name,
            canonical,
            description,
            capabilities,
            context_window,
            max_output,
            protocols,
            parameters,
            price_input,
            price_output,
            currency,
            status,
        },
        deployments,
    ))
}

/// 校验模型组 PATCH 请求。
pub fn validate_group_update(request: ModelGroupUpdateRequest) -> Result<GroupPatch, AppError> {
    let mut errors = BTreeMap::new();
    let mut patch = GroupPatch::default();

    if let Some(raw) = request.client_model_name {
        match validate_client_model_name(&raw) {
            Ok(value) => patch.client_model_name = Some(value),
            Err(message) => collect(&mut errors, "client_model_name", message),
        }
    }
    if let Some(raw) = request.canonical {
        match validate_canonical(&raw) {
            Ok(value) => patch.canonical = Some(value),
            Err(message) => collect(&mut errors, "canonical", message),
        }
    }
    if let Some(raw) = request.display_name {
        match validate_display_name(Some(&raw)) {
            Ok(value) => patch.display_name = Some(value),
            Err(message) => collect(&mut errors, "display_name", message),
        }
    }
    if let Some(raw) = request.description {
        match validate_optional_text(Some(&raw), "description", 500) {
            Ok(value) => patch.description = Some(value),
            Err(message) => collect(&mut errors, "description", message),
        }
    }
    if let Some(raw) = request.capabilities {
        match validate_capabilities(Some(&raw)) {
            Ok(value) => patch.capabilities = Some(value),
            Err(message) => collect(&mut errors, "capabilities", message),
        }
    }
    if let Some(raw) = request.protocols {
        match validate_protocols(Some(&raw)) {
            Ok(value) => patch.protocols = Some(value),
            Err(message) => collect(&mut errors, "protocols", message),
        }
    }
    if let Some(raw) = request.parameters {
        match validate_parameters(Some(raw)) {
            Ok(value) => patch.parameters = Some(value),
            Err(message) => collect(&mut errors, "parameters", message),
        }
    }
    if let Some(raw) = request.context_window {
        match validate_context_window(raw) {
            Ok(value) => patch.context_window = Some(value),
            Err(message) => collect(&mut errors, "context_window", message),
        }
    }
    if let Some(raw) = request.max_output {
        match validate_max_output(raw) {
            Ok(value) => patch.max_output = Some(value),
            Err(message) => collect(&mut errors, "max_output", message),
        }
    }
    if let Some(raw) = request.price_input {
        match validate_price(raw, "price_input") {
            Ok(value) => patch.price_input = Some(value),
            Err(message) => collect(&mut errors, "price_input", message),
        }
    }
    if let Some(raw) = request.price_output {
        match validate_price(raw, "price_output") {
            Ok(value) => patch.price_output = Some(value),
            Err(message) => collect(&mut errors, "price_output", message),
        }
    }
    if let Some(raw) = request.currency {
        match validate_currency(Some(&raw)) {
            Ok(value) => patch.currency = Some(value),
            Err(message) => collect(&mut errors, "currency", message),
        }
    }
    if let Some(raw) = request.status {
        match validate_status(Some(&raw), true) {
            Ok(value) => patch.status = Some(value),
            Err(message) => collect(&mut errors, "status", message),
        }
    }

    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    if patch.is_empty() {
        return Err(AppError::validation_field(
            "body",
            "至少提供一个要更新的字段",
        ));
    }
    Ok(patch)
}

pub fn apply_group_patch(base: &GroupFields, patch: &GroupPatch) -> GroupFields {
    let mut next = base.clone();
    if let Some(value) = &patch.client_model_name {
        next.client_model_name = value.clone();
    }
    if let Some(value) = &patch.display_name {
        next.display_name = value.clone();
    }
    if let Some(value) = &patch.canonical {
        next.canonical = value.clone();
    }
    if let Some(value) = &patch.description {
        next.description = value.clone();
    }
    if let Some(value) = &patch.capabilities {
        next.capabilities = value.clone();
    }
    if let Some(value) = patch.context_window {
        next.context_window = value;
    }
    if let Some(value) = patch.max_output {
        next.max_output = value;
    }
    if let Some(value) = &patch.protocols {
        next.protocols = value.clone();
    }
    if let Some(value) = &patch.parameters {
        next.parameters = value.clone();
    }
    if let Some(value) = patch.price_input {
        next.price_input = value;
    }
    if let Some(value) = patch.price_output {
        next.price_output = value;
    }
    if let Some(value) = &patch.currency {
        next.currency = value.clone();
    }
    if let Some(value) = &patch.status {
        next.status = value.clone();
    }
    next
}

fn validate_deployment_inputs(
    inputs: &[ModelGroupDeploymentInput],
) -> Result<Vec<DeploymentSpec>, BTreeMap<String, String>> {
    let mut errors = BTreeMap::new();
    if inputs.len() > MAX_NESTED_DEPLOYMENTS {
        collect(
            &mut errors,
            "deployments",
            format!("嵌套部署最多 {MAX_NESTED_DEPLOYMENTS} 条"),
        );
    }
    let mut seen = HashSet::new();
    let mut specs = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let key = (input.upstream_model_id, input.provider_account_id);
        if !seen.insert(key) {
            collect(&mut errors, "deployments", "同一上游模型与账号不能重复");
        }
        let priority = match validate_priority(input.priority) {
            Ok(value) => value,
            Err(message) => {
                collect(
                    &mut errors,
                    "deployments",
                    format!("第 {index} 条: {message}"),
                );
                MIN_PRIORITY
            }
        };
        let weight = match validate_weight(input.weight) {
            Ok(value) => value,
            Err(message) => {
                collect(
                    &mut errors,
                    "deployments",
                    format!("第 {index} 条: {message}"),
                );
                MAX_WEIGHT
            }
        };
        let notes = match validate_optional_text(input.notes.as_deref(), "notes", 500) {
            Ok(value) => value,
            Err(message) => {
                collect(
                    &mut errors,
                    "deployments",
                    format!("第 {index} 条: {message}"),
                );
                String::new()
            }
        };
        specs.push(DeploymentSpec {
            upstream_model_id: input.upstream_model_id,
            provider_account_id: input.provider_account_id,
            priority,
            weight,
            enabled: input.enabled.unwrap_or(true),
            notes,
        });
    }
    if errors.is_empty() {
        Ok(specs)
    } else {
        Err(errors)
    }
}

pub fn validate_deployment_create(
    request: DeploymentCreateRequest,
) -> Result<(DeploymentSpec, Option<Uuid>), AppError> {
    let mut errors = BTreeMap::new();
    let priority = match validate_priority(request.priority) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "priority", message);
            MIN_PRIORITY
        }
    };
    let weight = match validate_weight(request.weight) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "weight", message);
            MAX_WEIGHT
        }
    };
    let notes = match validate_optional_text(request.notes.as_deref(), "notes", 500) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "notes", message);
            String::new()
        }
    };
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    Ok((
        DeploymentSpec {
            upstream_model_id: request.upstream_model_id,
            provider_account_id: request.provider_account_id,
            priority,
            weight,
            enabled: request.enabled.unwrap_or(true),
            notes,
        },
        request.model_group_id,
    ))
}

pub fn validate_deployment_update(
    request: DeploymentUpdateRequest,
) -> Result<DeploymentPatch, AppError> {
    let mut errors = BTreeMap::new();
    let mut patch = DeploymentPatch {
        model_group_id: request.model_group_id,
        priority: None,
        weight: None,
        enabled: None,
        notes: None,
    };
    if let Some(raw) = request.priority {
        match validate_priority(Some(raw)) {
            Ok(value) => patch.priority = Some(value),
            Err(message) => collect(&mut errors, "priority", message),
        }
    }
    if let Some(raw) = request.weight {
        match validate_weight(Some(raw)) {
            Ok(value) => patch.weight = Some(value),
            Err(message) => collect(&mut errors, "weight", message),
        }
    }
    if let Some(raw) = request.notes {
        match validate_optional_text(Some(&raw), "notes", 500) {
            Ok(value) => patch.notes = Some(value),
            Err(message) => collect(&mut errors, "notes", message),
        }
    }
    patch.enabled = request.enabled;
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    if patch.is_empty() {
        return Err(AppError::validation_field(
            "body",
            "至少提供一个要更新的字段",
        ));
    }
    Ok(patch)
}

// ---------- warnings（纯函数，确定性输出） ----------

fn merge_warnings(lists: impl IntoIterator<Item = Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = lists.into_iter().flatten().collect();
    out.sort();
    out.dedup();
    out
}

/// 部署级 warnings：provider/账号停用、账号不健康、价格缺失、未入组。
pub fn deployment_warnings(
    provider_status: &str,
    account_status: &str,
    account_health: &str,
    price_missing: bool,
    ungrouped: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    if provider_status == "disabled" {
        out.push("provider_disabled".to_string());
    }
    if account_status == "disabled" {
        out.push("account_disabled".to_string());
    }
    if account_health == "unhealthy" {
        out.push("account_unhealthy".to_string());
    }
    if price_missing {
        out.push("price_missing".to_string());
    }
    if ungrouped {
        out.push("ungrouped".to_string());
    }
    merge_warnings([out])
}

/// 组级 warnings：归档、无启用部署、任一部署缺价。
pub fn group_warnings(
    status: &str,
    enabled_deployment_count: i64,
    any_deployment_price_missing: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    if status == GROUP_STATUS_ARCHIVED {
        out.push("archived".to_string());
    }
    if enabled_deployment_count == 0 {
        out.push("no_enabled_deployment".to_string());
    }
    if any_deployment_price_missing {
        out.push("price_missing".to_string());
    }
    merge_warnings([out])
}

/// 路由预览 warnings：组状态 + 汇总候选 warnings。
pub fn preview_warnings(group_status: &str, candidates: &[RoutePreviewCandidate]) -> Vec<String> {
    let mut out = Vec::new();
    if group_status == GROUP_STATUS_ARCHIVED {
        out.push("group_archived".to_string());
    }
    if !candidates.iter().any(|candidate| candidate.enabled) {
        out.push("no_enabled_deployment".to_string());
    }
    let candidate_warnings = candidates
        .iter()
        .map(|candidate| candidate.warnings.clone())
        .collect::<Vec<_>>();
    out.extend(merge_warnings(candidate_warnings));
    merge_warnings([out])
}

/// 从 JSONB 读取字符串数组（异常值回退空数组，避免 500）。
pub fn string_array(value: &Value) -> Vec<String> {
    serde_json::from_value::<Vec<String>>(value.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn create_request(value: serde_json::Value) -> ModelGroupCreateRequest {
        serde_json::from_value(value).expect("create request")
    }

    #[test]
    fn client_model_name_rules() {
        assert_eq!(
            validate_client_model_name("  claude-sonnet-4-5 ").unwrap(),
            "claude-sonnet-4-5"
        );
        assert!(validate_client_model_name("").is_err());
        assert!(validate_client_model_name("-bad").is_err());
        assert!(validate_client_model_name("has space").is_err());
        assert!(validate_client_model_name("with/slash").is_err());
        assert!(validate_client_model_name(&"a".repeat(65)).is_err());
        assert!(validate_client_model_name("ok.name_1:x-y").is_ok());
    }

    #[test]
    fn canonical_rules() {
        assert_eq!(
            validate_canonical(" claude-sonnet-4-5 ").unwrap(),
            "claude-sonnet-4-5"
        );
        assert!(validate_canonical("").is_err());
        assert!(validate_canonical("a b").is_err());
        assert!(validate_canonical(&"a".repeat(101)).is_err());
    }

    #[test]
    fn capabilities_dedup_and_enum() {
        let value = validate_capabilities(Some(&[
            "chat".to_string(),
            "chat".to_string(),
            "tools".to_string(),
        ]))
        .unwrap();
        assert_eq!(value, vec!["chat".to_string(), "tools".to_string()]);
        assert!(validate_capabilities(Some(&["Chat".to_string()])).is_err());
        assert!(validate_capabilities(Some(&["-bad".to_string()])).is_err());
    }

    #[test]
    fn protocols_enum_and_dedup() {
        assert_eq!(
            validate_protocols(Some(&[
                "openai_chat".to_string(),
                "openai_chat".to_string()
            ]))
            .unwrap(),
            vec!["openai_chat".to_string()]
        );
        assert!(validate_protocols(Some(&["grpc".to_string()])).is_err());
    }

    #[test]
    fn parameters_must_be_object_and_bounded() {
        assert!(validate_parameters(Some(json!({"temperature": true}))).is_ok());
        assert!(validate_parameters(Some(json!([1, 2]))).is_err());
        let big = json!({"k": "x".repeat(MAX_PARAMETERS_BYTES)});
        assert!(validate_parameters(Some(big)).is_err());
    }

    #[test]
    fn price_and_currency_rules() {
        assert_eq!(
            validate_price(Some(0.003), "price_input").unwrap(),
            Some(0.003)
        );
        assert!(validate_price(Some(-1.0), "price_input").is_err());
        assert!(validate_price(Some(f64::INFINITY), "price_input").is_err());
        assert_eq!(validate_currency(Some("usd")).unwrap(), "USD");
        assert!(validate_currency(Some("EUR")).is_err());
    }

    #[test]
    fn context_pair_rules() {
        assert!(check_context_pair(Some(2000), Some(1000)).is_ok());
        assert!(check_context_pair(Some(1000), Some(2000)).is_err());
        assert!(check_context_pair(None, Some(2000)).is_ok());
    }

    #[test]
    fn group_create_validation_collects_errors() {
        let error = validate_group_create(create_request(json!({
            "client_model_name": "-bad",
            "canonical": "has space",
            "capabilities": ["Chat"],
            "protocols": ["grpc"],
            "currency": "EUR",
            "price_input": -1.0
        })))
        .unwrap_err();
        match error {
            AppError::Validation { errors } => {
                for field in [
                    "client_model_name",
                    "canonical",
                    "capabilities",
                    "protocols",
                    "currency",
                    "price_input",
                ] {
                    assert!(errors.contains_key(field), "missing {field}: {errors:?}");
                }
            }
            other => panic!("expected validation, got {other:?}"),
        }
    }

    #[test]
    fn group_create_defaults() {
        let (fields, deployments) = validate_group_create(create_request(json!({
            "client_model_name": "deepseek-chat",
            "canonical": "deepseek-chat"
        })))
        .unwrap();
        assert_eq!(fields.status, GROUP_STATUS_DRAFT);
        assert_eq!(fields.currency, "CNY");
        assert_eq!(fields.display_name, "");
        assert!(fields.capabilities.is_empty());
        assert!(deployments.is_empty());
        assert!(matches!(fields.parameters, Value::Object(_)));
    }

    #[test]
    fn group_create_active_requires_enabled_deployment() {
        let error = validate_group_create(create_request(json!({
            "client_model_name": "m",
            "canonical": "m",
            "status": "active"
        })))
        .unwrap_err();
        assert!(matches!(
            error,
            AppError::BadRequest {
                code: "no_enabled_deployment",
                ..
            }
        ));

        let error = validate_group_create(create_request(json!({
            "client_model_name": "m",
            "canonical": "m",
            "status": "active",
            "deployments": [{
                "upstream_model_id": "00000000-0000-0000-0000-000000000001",
                "provider_account_id": "00000000-0000-0000-0000-000000000002",
                "enabled": false
            }]
        })))
        .unwrap_err();
        assert!(matches!(
            error,
            AppError::BadRequest {
                code: "no_enabled_deployment",
                ..
            }
        ));
    }

    #[test]
    fn nested_deployments_duplicate_and_ranges() {
        let error = validate_group_create(create_request(json!({
            "client_model_name": "m",
            "canonical": "m",
            "deployments": [
                {"upstream_model_id": "00000000-0000-0000-0000-000000000001",
                 "provider_account_id": "00000000-0000-0000-0000-000000000002",
                 "priority": 0, "weight": 101},
                {"upstream_model_id": "00000000-0000-0000-0000-000000000001",
                 "provider_account_id": "00000000-0000-0000-0000-000000000002"}
            ]
        })))
        .unwrap_err();
        assert!(matches!(error, AppError::Validation { .. }));
    }

    #[test]
    fn group_update_nullable_vs_missing() {
        let patch = validate_group_update(
            serde_json::from_value(json!({"display_name": "Claude Sonnet 4.5"})).expect("update"),
        )
        .unwrap();
        assert!(patch.context_window.is_none());

        let patch = validate_group_update(
            serde_json::from_value(json!({"context_window": null, "max_output": 512}))
                .expect("update"),
        )
        .unwrap();
        assert_eq!(patch.context_window, Some(None));
        assert_eq!(patch.max_output, Some(Some(512)));
    }

    #[test]
    fn group_update_empty_body_rejected() {
        let error = validate_group_update(serde_json::from_value(json!({})).expect("empty update"))
            .unwrap_err();
        assert!(matches!(error, AppError::Validation { .. }));
    }

    #[test]
    fn deployment_update_model_group_null_vs_missing() {
        let patch = validate_deployment_update(
            serde_json::from_value(json!({"model_group_id": null})).expect("detach"),
        )
        .unwrap();
        assert_eq!(patch.model_group_id, Some(None));

        let patch = validate_deployment_update(
            serde_json::from_value(json!({"enabled": false})).expect("disable"),
        )
        .unwrap();
        assert!(patch.model_group_id.is_none());
        assert_eq!(patch.enabled, Some(false));
    }

    #[test]
    fn page_params_bounds() {
        assert_eq!(validate_page(None, None).unwrap().limit, DEFAULT_PAGE_LIMIT);
        assert!(validate_page(Some(0), None).is_err());
        assert!(validate_page(Some(201), None).is_err());
        assert!(validate_page(None, Some(-1)).is_err());
    }

    #[test]
    fn deployment_warnings_rules() {
        assert_eq!(
            deployment_warnings("disabled", "disabled", "unhealthy", true, true),
            vec![
                "account_disabled".to_string(),
                "account_unhealthy".to_string(),
                "price_missing".to_string(),
                "provider_disabled".to_string(),
                "ungrouped".to_string(),
            ]
        );
        assert!(deployment_warnings("active", "active", "healthy", false, false).is_empty());
    }

    #[test]
    fn group_warnings_rules() {
        assert_eq!(
            group_warnings(GROUP_STATUS_ARCHIVED, 0, true),
            vec![
                "archived".to_string(),
                "no_enabled_deployment".to_string(),
                "price_missing".to_string(),
            ]
        );
        assert!(group_warnings(GROUP_STATUS_ACTIVE, 1, false).is_empty());
    }

    #[test]
    fn preview_warnings_union_is_deterministic() {
        let candidates = vec![RoutePreviewCandidate {
            deployment_id: Uuid::nil(),
            upstream_model_id: Uuid::nil(),
            upstream_model: "m".to_string(),
            provider_id: Uuid::nil(),
            provider_name: "p".to_string(),
            provider_status: "disabled".to_string(),
            provider_account_id: Uuid::nil(),
            account_name: "a".to_string(),
            account_status: "disabled".to_string(),
            account_health: "unhealthy".to_string(),
            priority: 1,
            weight: 100,
            enabled: true,
            warnings: vec!["provider_disabled".to_string()],
        }];
        assert_eq!(
            preview_warnings(GROUP_STATUS_ACTIVE, &candidates),
            vec!["provider_disabled".to_string()]
        );
        assert_eq!(
            preview_warnings(GROUP_STATUS_ARCHIVED, &[]),
            vec![
                "group_archived".to_string(),
                "no_enabled_deployment".to_string()
            ]
        );
    }

    #[test]
    fn string_array_tolerates_invalid_json() {
        assert_eq!(string_array(&json!(["a", "b"])), vec!["a", "b"]);
        assert!(string_array(&json!({"a": 1})).is_empty());
        assert!(string_array(&json!(null)).is_empty());
    }

    #[test]
    fn apply_group_patch_merges_nullable_fields() {
        let base = GroupFields {
            client_model_name: "m".to_string(),
            display_name: String::new(),
            canonical: "m".to_string(),
            description: String::new(),
            capabilities: vec!["chat".to_string()],
            context_window: Some(1000),
            max_output: Some(100),
            protocols: Vec::new(),
            parameters: Value::Object(serde_json::Map::new()),
            price_input: Some(1.0),
            price_output: None,
            currency: "CNY".to_string(),
            status: GROUP_STATUS_DRAFT.to_string(),
        };
        let patch = GroupPatch {
            context_window: Some(None),
            price_output: Some(Some(2.0)),
            status: Some(GROUP_STATUS_ACTIVE.to_string()),
            ..GroupPatch::default()
        };
        let next = apply_group_patch(&base, &patch);
        assert_eq!(next.context_window, None);
        assert_eq!(next.price_output, Some(2.0));
        assert_eq!(next.status, GROUP_STATUS_ACTIVE);
        assert_eq!(next.capabilities, base.capabilities);
    }
}

// ===========================================================================
// 发布（Publication）DTO / 校验（迭代 2）
// ===========================================================================

pub const ALL_SUBJECT_TYPES: [&str; 5] =
    ["all_users", "user", "user_group", "application", "device"];
/// v0.1.x 可写的发布对象类型（其余预留，写 API 拒绝）。
pub const WRITABLE_SUBJECT_TYPES: [&str; 3] = ["all_users", "user", "user_group"];
pub const MAX_PUBLICATION_TARGETS: usize = 500;
pub const MIN_DATA_LEVEL: i32 = 1;
pub const MAX_DATA_LEVEL: i32 = 4;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationListQuery {
    #[serde(default)]
    pub model_group_id: Option<Uuid>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub subject_type: Option<String>,
    #[serde(default)]
    pub subject_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationTargetInput {
    pub subject_type: String,
    #[serde(default)]
    pub subject_id: Option<Uuid>,
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub allowed: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationCreateRequest {
    pub model_group_id: Uuid,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub data_level_max: Option<i32>,
    #[serde(default)]
    pub quota_rpm: Option<i32>,
    #[serde(default)]
    pub quota_tpm: Option<i64>,
    #[serde(default)]
    pub quota_daily_requests: Option<i64>,
    #[serde(default)]
    pub note: Option<String>,
    pub targets: Vec<PublicationTargetInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationUpdateRequest {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub data_level_max: Option<Option<i32>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub quota_rpm: Option<Option<i32>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub quota_tpm: Option<Option<i64>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub quota_daily_requests: Option<Option<i64>>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub targets: Option<Vec<PublicationTargetInput>>,
}

#[derive(Debug, Clone)]
pub struct PublicationTargetSpec {
    pub subject_type: String,
    pub subject_id: Option<Uuid>,
    pub visible: bool,
    pub allowed: bool,
}

#[derive(Debug, Clone)]
pub struct PublicationFields {
    pub model_group_id: Uuid,
    pub status: String,
    pub data_level_max: Option<i16>,
    pub quota_rpm: Option<i32>,
    pub quota_tpm: Option<i64>,
    pub quota_daily_requests: Option<i64>,
    pub note: String,
    pub targets: Vec<PublicationTargetSpec>,
}

#[derive(Debug, Default, Clone)]
pub struct PublicationPatch {
    pub status: Option<String>,
    pub data_level_max: Option<Option<i16>>,
    pub quota_rpm: Option<Option<i32>>,
    pub quota_tpm: Option<Option<i64>>,
    pub quota_daily_requests: Option<Option<i64>>,
    pub note: Option<String>,
    pub targets: Option<Vec<PublicationTargetSpec>>,
}

impl PublicationPatch {
    pub fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.data_level_max.is_none()
            && self.quota_rpm.is_none()
            && self.quota_tpm.is_none()
            && self.quota_daily_requests.is_none()
            && self.note.is_none()
            && self.targets.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Enforcement {
    pub visible: &'static str,
    pub allowed: &'static str,
    pub quota: &'static str,
    pub data_level: &'static str,
}

impl Default for Enforcement {
    fn default() -> Self {
        Self {
            visible: "enforced",
            allowed: "enforced",
            quota: "config_only",
            data_level: "config_only",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicationTargetAdmin {
    pub id: Uuid,
    pub subject_type: String,
    pub subject_id: Option<Uuid>,
    pub subject_label: Option<String>,
    pub visible: bool,
    pub allowed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicationAdmin {
    pub id: Uuid,
    pub model_group_id: Uuid,
    pub model_group_client_name: String,
    pub status: String,
    pub data_level_max: Option<i16>,
    pub quota_rpm: Option<i32>,
    pub quota_tpm: Option<i64>,
    pub quota_daily_requests: Option<i64>,
    pub note: String,
    pub created_by: Option<Uuid>,
    pub targets: Vec<PublicationTargetAdmin>,
    pub enforcement: Enforcement,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientModelDetail {
    #[serde(flatten)]
    pub model: ClientModel,
    pub parameters: Value,
    pub catalog_version: String,
}

// ---------- 发布 / 目录解析的纯逻辑（可单测，无 IO） ----------

/// 发布对象解析视图（v0.1.x 只有 all_users/user 生效，其余类型 fail-closed）。
#[derive(Debug, Clone)]
pub struct TargetView {
    pub subject_type: String,
    pub subject_id: Option<Uuid>,
    pub visible: bool,
    pub allowed: bool,
}

#[derive(Debug, Clone)]
pub struct CatalogSubject {
    pub user_id: Uuid,
    /// device 发布对象为 v0.1.x 预留（解析 fail-closed）；保留字段供后续启用。
    #[allow(dead_code)]
    pub device_id: Option<Uuid>,
    /// 用户所属用户组（access `AuthContext.user_group_ids`；user_group 发布对象/工具默认用）。
    pub user_group_ids: Vec<Uuid>,
    pub key_allowed_models: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAuthorization {
    Allowed,
    VisibleOnly,
    NotPublished,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogError {
    DatabaseUnavailable,
}

#[derive(Debug, Clone, Copy)]
pub struct EffectivePublication {
    pub visible: bool,
    pub allowed: bool,
}

/// 生效发布解析：最具体层生效（user > user_group > all_users），层内并集；Key 白名单叠加收窄。
/// 返回 `None` = 无匹配对象（未发布）。`user_group` 取 subject 所属组并集；
/// 预留类型（application/device）仍 fail-closed。
pub fn effective_publication(
    targets: &[TargetView],
    subject: &CatalogSubject,
    client_model_name: &str,
) -> Option<EffectivePublication> {
    effective_publication_with_match(targets, subject, client_model_name)
        .map(|(effective, _)| effective)
}

/// 与 [`effective_publication`] 相同，但额外返回命中的发布对象层级（`user`/`all_users`），
/// 供管理台员工目录预览的 `matched_by` 诊断字段使用。
pub fn effective_publication_with_match(
    targets: &[TargetView],
    subject: &CatalogSubject,
    client_model_name: &str,
) -> Option<(EffectivePublication, &'static str)> {
    let mut user_rows: Vec<&TargetView> = Vec::new();
    let mut user_group_rows: Vec<&TargetView> = Vec::new();
    let mut all_rows: Vec<&TargetView> = Vec::new();
    for target in targets {
        match target.subject_type.as_str() {
            "user" => {
                if target.subject_id == Some(subject.user_id) {
                    user_rows.push(target);
                }
            }
            "user_group" => {
                if target
                    .subject_id
                    .is_some_and(|group_id| subject.user_group_ids.contains(&group_id))
                {
                    user_group_rows.push(target);
                }
            }
            "all_users" => all_rows.push(target),
            _ => {}
        }
    }
    // 层级：user > user_group（多组并集） > all_users。
    let (rows, matched_by) = if !user_rows.is_empty() {
        (user_rows, "user")
    } else if !user_group_rows.is_empty() {
        (user_group_rows, "user_group")
    } else if !all_rows.is_empty() {
        (all_rows, "all_users")
    } else {
        return None;
    };
    let mut visible = rows.iter().any(|row| row.visible);
    let mut allowed = rows.iter().any(|row| row.allowed);
    if !subject.key_allowed_models.is_empty()
        && !subject
            .key_allowed_models
            .iter()
            .any(|allowed| allowed == client_model_name)
    {
        visible = false;
        allowed = false;
    }
    Some((EffectivePublication { visible, allowed }, matched_by))
}

/// 组能力为空时按启用部署的上游能力取交集（保守，不夸大）。
pub fn derive_capabilities(group: &[String], deployment_caps: &[Vec<String>]) -> Vec<String> {
    if !group.is_empty() {
        return group.to_vec();
    }
    let mut iter = deployment_caps.iter();
    let Some(first) = iter.next() else {
        return Vec::new();
    };
    let mut derived: Vec<String> = first.clone();
    for caps in iter {
        derived.retain(|item| caps.contains(item));
    }
    derived.sort();
    derived.dedup();
    derived
}

/// 组未配置时按启用部署的上游值取最小值；任一未知则为 None（保守）。
pub fn derive_min_metric(group: Option<i32>, values: &[Option<i32>]) -> Option<i32> {
    if group.is_some() {
        return group;
    }
    if values.is_empty() {
        return None;
    }
    let mut min_value: Option<i32> = None;
    for value in values {
        let Some(value) = value else {
            return None;
        };
        min_value = Some(min_value.map_or(*value, |current| current.min(*value)));
    }
    min_value
}

// ---------- 发布校验 ----------

fn validate_subject_type_for_filter(raw: &str) -> Result<String, AppError> {
    if ALL_SUBJECT_TYPES.contains(&raw) {
        Ok(raw.to_string())
    } else {
        Err(AppError::validation_field(
            "subject_type",
            format!("subject_type 必须是 {} 之一", ALL_SUBJECT_TYPES.join("/")),
        ))
    }
}

fn validate_publication_status(raw: Option<&str>) -> Result<String, AppError> {
    let status = raw.unwrap_or("active").trim();
    if status == "active" || status == "paused" {
        Ok(status.to_string())
    } else {
        Err(AppError::validation_field(
            "status",
            "status 仅支持 active/paused",
        ))
    }
}

fn validate_data_level(raw: Option<i32>) -> Result<Option<i16>, String> {
    match raw {
        None => Ok(None),
        Some(value) if (MIN_DATA_LEVEL..=MAX_DATA_LEVEL).contains(&value) => Ok(Some(value as i16)),
        Some(_) => Err(format!(
            "data_level_max 必须在 {MIN_DATA_LEVEL}–{MAX_DATA_LEVEL}"
        )),
    }
}

fn validate_positive_i32(raw: Option<i32>, field: &str) -> Result<Option<i32>, String> {
    match raw {
        None => Ok(None),
        Some(value) if value > 0 => Ok(Some(value)),
        Some(_) => Err(format!("{field} 必须大于 0")),
    }
}

fn validate_positive_i64(raw: Option<i64>, field: &str) -> Result<Option<i64>, String> {
    match raw {
        None => Ok(None),
        Some(value) if value > 0 => Ok(Some(value)),
        Some(_) => Err(format!("{field} 必须大于 0")),
    }
}

fn validate_publication_targets(
    inputs: &[PublicationTargetInput],
) -> Result<Vec<PublicationTargetSpec>, AppError> {
    if inputs.is_empty() {
        return Err(AppError::validation_field(
            "targets",
            "至少提供一个发布对象",
        ));
    }
    if inputs.len() > MAX_PUBLICATION_TARGETS {
        return Err(AppError::validation_field(
            "targets",
            format!("发布对象最多 {MAX_PUBLICATION_TARGETS} 个"),
        ));
    }
    let mut seen = HashSet::new();
    let mut all_users_count = 0_usize;
    let mut specs = Vec::with_capacity(inputs.len());
    for input in inputs {
        let subject_type = input.subject_type.trim();
        if !ALL_SUBJECT_TYPES.contains(&subject_type) {
            return Err(AppError::validation_field(
                "targets",
                format!("subject_type 必须是 {} 之一", ALL_SUBJECT_TYPES.join("/")),
            ));
        }
        if !WRITABLE_SUBJECT_TYPES.contains(&subject_type) {
            return Err(AppError::bad_request(
                "target_type_not_supported",
                "该发布对象类型 v0.1.x 预留（仅支持 all_users / user / user_group）",
            ));
        }
        let (subject_id, visible, allowed) = match subject_type {
            "all_users" => {
                if input.subject_id.is_some() {
                    return Err(AppError::validation_field(
                        "targets",
                        "all_users 的 subject_id 必须为空",
                    ));
                }
                all_users_count += 1;
                if all_users_count > 1 {
                    return Err(AppError::validation_field(
                        "targets",
                        "all_users 只能出现一次",
                    ));
                }
                (
                    None,
                    input.visible.unwrap_or(true),
                    input.allowed.unwrap_or(true),
                )
            }
            "user" | "user_group" => {
                let Some(subject_id) = input.subject_id else {
                    return Err(AppError::validation_field(
                        "targets",
                        format!("{subject_type} 发布对象必须提供 subject_id"),
                    ));
                };
                (
                    Some(subject_id),
                    input.visible.unwrap_or(true),
                    input.allowed.unwrap_or(true),
                )
            }
            _ => unreachable!("unsupported subject type filtered above"),
        };
        if allowed && !visible {
            return Err(AppError::validation_field(
                "targets",
                "allowed=true 时 visible 不能为 false",
            ));
        }
        let key = (subject_type.to_string(), subject_id);
        if !seen.insert(key) {
            return Err(AppError::validation_field(
                "targets",
                "同一发布对象不能重复",
            ));
        }
        specs.push(PublicationTargetSpec {
            subject_type: subject_type.to_string(),
            subject_id,
            visible,
            allowed,
        });
    }
    Ok(specs)
}

pub fn validate_publication_create(
    request: PublicationCreateRequest,
) -> Result<PublicationFields, AppError> {
    let mut errors = BTreeMap::new();
    let status = match validate_publication_status(request.status.as_deref()) {
        Ok(value) => value,
        Err(error) => {
            if let AppError::Validation {
                errors: field_errors,
            } = &error
            {
                for (field, message) in field_errors {
                    collect(&mut errors, field, message.clone());
                }
            }
            "active".to_string()
        }
    };
    let data_level_max = match validate_data_level(request.data_level_max) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "data_level_max", message);
            None
        }
    };
    let quota_rpm = match validate_positive_i32(request.quota_rpm, "quota_rpm") {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "quota_rpm", message);
            None
        }
    };
    let quota_tpm = match validate_positive_i64(request.quota_tpm, "quota_tpm") {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "quota_tpm", message);
            None
        }
    };
    let quota_daily_requests =
        match validate_positive_i64(request.quota_daily_requests, "quota_daily_requests") {
            Ok(value) => value,
            Err(message) => {
                collect(&mut errors, "quota_daily_requests", message);
                None
            }
        };
    let note = match validate_optional_text(request.note.as_deref(), "note", 500) {
        Ok(value) => value,
        Err(message) => {
            collect(&mut errors, "note", message);
            String::new()
        }
    };
    let targets = match validate_publication_targets(&request.targets) {
        Ok(value) => value,
        Err(error) => {
            if let AppError::Validation {
                errors: field_errors,
            } = &error
            {
                for (field, message) in field_errors {
                    collect(&mut errors, field, message.clone());
                }
            }
            return Err(error);
        }
    };
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    Ok(PublicationFields {
        model_group_id: request.model_group_id,
        status,
        data_level_max,
        quota_rpm,
        quota_tpm,
        quota_daily_requests,
        note,
        targets,
    })
}

pub fn validate_publication_update(
    request: PublicationUpdateRequest,
) -> Result<PublicationPatch, AppError> {
    let mut patch = PublicationPatch::default();
    if let Some(raw) = request.status {
        patch.status = Some(validate_publication_status(Some(&raw))?);
    }
    if let Some(raw) = request.data_level_max {
        patch.data_level_max = Some(
            validate_data_level(raw)
                .map_err(|message| AppError::validation_field("data_level_max", message))?,
        );
    }
    if let Some(raw) = request.quota_rpm {
        patch.quota_rpm = Some(
            validate_positive_i32(raw, "quota_rpm")
                .map_err(|message| AppError::validation_field("quota_rpm", message))?,
        );
    }
    if let Some(raw) = request.quota_tpm {
        patch.quota_tpm = Some(
            validate_positive_i64(raw, "quota_tpm")
                .map_err(|message| AppError::validation_field("quota_tpm", message))?,
        );
    }
    if let Some(raw) = request.quota_daily_requests {
        patch.quota_daily_requests = Some(
            validate_positive_i64(raw, "quota_daily_requests")
                .map_err(|message| AppError::validation_field("quota_daily_requests", message))?,
        );
    }
    if let Some(raw) = request.note {
        patch.note = Some(
            validate_optional_text(Some(&raw), "note", 500)
                .map_err(|message| AppError::validation_field("note", message))?,
        );
    }
    if let Some(raw) = request.targets {
        patch.targets = Some(validate_publication_targets(&raw)?);
    }
    if patch.is_empty() {
        return Err(AppError::validation_field(
            "body",
            "至少提供一个要更新的字段",
        ));
    }
    Ok(patch)
}

pub fn validate_publication_list_query(
    query: &PublicationListQuery,
) -> Result<(Option<String>, Option<String>), AppError> {
    let status = match query.status.as_deref() {
        None => None,
        Some(value) if value == "active" || value == "paused" => Some(value.to_string()),
        Some(_) => {
            return Err(AppError::validation_field(
                "status",
                "status 仅支持 active/paused",
            ));
        }
    };
    let subject_type = match query.subject_type.as_deref() {
        None => None,
        Some(value) => Some(validate_subject_type_for_filter(value)?),
    };
    Ok((status, subject_type))
}

#[cfg(test)]
mod publication_tests {
    use serde_json::json;

    use super::*;

    fn subject(user_id: Uuid, whitelist: &[&str]) -> CatalogSubject {
        CatalogSubject {
            user_id,
            device_id: None,
            user_group_ids: Vec::new(),
            key_allowed_models: whitelist.iter().map(|item| item.to_string()).collect(),
        }
    }

    fn subject_with_groups(user_id: Uuid, group_ids: &[Uuid]) -> CatalogSubject {
        CatalogSubject {
            user_id,
            device_id: None,
            user_group_ids: group_ids.to_vec(),
            key_allowed_models: Vec::new(),
        }
    }

    fn target(
        subject_type: &str,
        subject_id: Option<Uuid>,
        visible: bool,
        allowed: bool,
    ) -> TargetView {
        TargetView {
            subject_type: subject_type.to_string(),
            subject_id,
            visible,
            allowed,
        }
    }

    #[test]
    fn effective_all_users_then_user_override() {
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        let targets = vec![
            target("all_users", None, true, false),
            target("user", Some(alice), true, true),
        ];
        let alice_eff = effective_publication(&targets, &subject(alice, &[]), "m").unwrap();
        assert!(alice_eff.visible && alice_eff.allowed);
        let bob_eff = effective_publication(&targets, &subject(bob, &[]), "m").unwrap();
        assert!(bob_eff.visible && !bob_eff.allowed);
    }

    #[test]
    fn effective_user_tier_overrides_all_users_even_when_denied() {
        let alice = Uuid::new_v4();
        let targets = vec![
            target("all_users", None, true, true),
            target("user", Some(alice), true, false),
        ];
        let eff = effective_publication(&targets, &subject(alice, &[]), "m").unwrap();
        assert!(eff.visible && !eff.allowed);
    }

    #[test]
    fn effective_no_match_is_none_and_reserved_types_fail_closed() {
        let alice = Uuid::new_v4();
        let targets = vec![
            target("application", Some(Uuid::new_v4()), true, true),
            target("device", Some(Uuid::new_v4()), true, true),
        ];
        assert!(effective_publication(&targets, &subject(alice, &[]), "m").is_none());
    }

    #[test]
    fn user_group_tier_union_and_matched_by() {
        let alice = Uuid::new_v4();
        let group_a = Uuid::new_v4();
        let group_b = Uuid::new_v4();
        let targets = vec![
            target("all_users", None, true, false),
            target("user_group", Some(group_a), true, false),
            target("user_group", Some(group_b), false, true),
        ];
        // 组内并集：可见来自 A，可用来自 B。
        let (effective, matched) = effective_publication_with_match(
            &targets,
            &subject_with_groups(alice, &[group_a, group_b]),
            "m",
        )
        .expect("group tier");
        assert_eq!(matched, "user_group");
        assert!(effective.visible && effective.allowed);

        // 非成员：回落到 all_users。
        let (effective, matched) = effective_publication_with_match(
            &targets,
            &subject_with_groups(alice, &[Uuid::new_v4()]),
            "m",
        )
        .expect("all_users tier");
        assert_eq!(matched, "all_users");
        assert!(effective.visible && !effective.allowed);
    }

    #[test]
    fn user_tier_still_overrides_user_group() {
        let alice = Uuid::new_v4();
        let group = Uuid::new_v4();
        let targets = vec![
            target("user_group", Some(group), true, true),
            target("user", Some(alice), true, false),
        ];
        let (effective, matched) =
            effective_publication_with_match(&targets, &subject_with_groups(alice, &[group]), "m")
                .unwrap();
        assert_eq!(matched, "user");
        assert!(effective.visible && !effective.allowed);
    }

    #[test]
    fn key_whitelist_narrows_visibility() {
        let alice = Uuid::new_v4();
        let targets = vec![target("all_users", None, true, true)];
        let eff = effective_publication(&targets, &subject(alice, &["other"]), "m").unwrap();
        assert!(!eff.visible && !eff.allowed);
        let eff = effective_publication(&targets, &subject(alice, &["m"]), "m").unwrap();
        assert!(eff.visible && eff.allowed);
    }

    #[test]
    fn capabilities_intersection_when_group_empty() {
        let caps = vec![
            vec!["chat".to_string(), "tools".to_string()],
            vec!["chat".to_string(), "vision".to_string()],
        ];
        assert_eq!(derive_capabilities(&[], &caps), vec!["chat".to_string()]);
        assert_eq!(
            derive_capabilities(&["chat".to_string()], &caps),
            vec!["chat".to_string()]
        );
        assert!(derive_capabilities(&[], &[]).is_empty());
    }

    #[test]
    fn min_metric_is_conservative() {
        assert_eq!(derive_min_metric(Some(1000), &[Some(10)]), Some(1000));
        assert_eq!(derive_min_metric(None, &[Some(100), Some(50)]), Some(50));
        assert_eq!(derive_min_metric(None, &[Some(100), None]), None);
        assert_eq!(derive_min_metric(None, &[]), None);
    }

    #[test]
    fn publication_targets_validation() {
        // all_users 单条
        let targets = vec![PublicationTargetInput {
            subject_type: "all_users".to_string(),
            subject_id: None,
            visible: None,
            allowed: None,
        }];
        assert_eq!(validate_publication_targets(&targets).unwrap().len(), 1);

        // user_group 已支持：subject_id 必填
        let group_target = vec![PublicationTargetInput {
            subject_type: "user_group".to_string(),
            subject_id: Some(Uuid::new_v4()),
            visible: None,
            allowed: None,
        }];
        let specs = validate_publication_targets(&group_target).unwrap();
        assert_eq!(specs[0].subject_type, "user_group");
        let missing_group_id = vec![PublicationTargetInput {
            subject_type: "user_group".to_string(),
            subject_id: None,
            visible: None,
            allowed: None,
        }];
        assert!(validate_publication_targets(&missing_group_id).is_err());

        // 预留类型（application/device）-> target_type_not_supported
        let reserved = vec![PublicationTargetInput {
            subject_type: "application".to_string(),
            subject_id: Some(Uuid::new_v4()),
            visible: None,
            allowed: None,
        }];
        assert!(matches!(
            validate_publication_targets(&reserved).unwrap_err(),
            AppError::BadRequest {
                code: "target_type_not_supported",
                ..
            }
        ));

        // allowed=true 但 visible=false
        let bad = vec![PublicationTargetInput {
            subject_type: "user".to_string(),
            subject_id: Some(Uuid::new_v4()),
            visible: Some(false),
            allowed: Some(true),
        }];
        assert!(validate_publication_targets(&bad).is_err());

        // 重复对象
        let id = Uuid::new_v4();
        let dup = vec![
            PublicationTargetInput {
                subject_type: "user".to_string(),
                subject_id: Some(id),
                visible: None,
                allowed: None,
            },
            PublicationTargetInput {
                subject_type: "user".to_string(),
                subject_id: Some(id),
                visible: None,
                allowed: None,
            },
        ];
        assert!(validate_publication_targets(&dup).is_err());

        // 空 targets / all_users 重复
        assert!(validate_publication_targets(&[]).is_err());
        let multi_all = vec![
            PublicationTargetInput {
                subject_type: "all_users".to_string(),
                subject_id: None,
                visible: None,
                allowed: None,
            },
            PublicationTargetInput {
                subject_type: "all_users".to_string(),
                subject_id: None,
                visible: None,
                allowed: None,
            },
        ];
        assert!(validate_publication_targets(&multi_all).is_err());
    }

    #[test]
    fn publication_create_validation() {
        let request: PublicationCreateRequest = serde_json::from_value(json!({
            "model_group_id": Uuid::new_v4(),
            "data_level_max": 9,
            "quota_rpm": 0,
            "targets": [{"subject_type": "all_users"}]
        }))
        .unwrap();
        let error = validate_publication_create(request).unwrap_err();
        match error {
            AppError::Validation { errors } => {
                assert!(errors.contains_key("data_level_max"));
                assert!(errors.contains_key("quota_rpm"));
            }
            other => panic!("expected validation, got {other:?}"),
        }
    }

    #[test]
    fn publication_update_nullable_and_empty() {
        let patch = validate_publication_update(
            serde_json::from_value(json!({"data_level_max": null, "quota_tpm": 100}))
                .expect("update"),
        )
        .unwrap();
        assert_eq!(patch.data_level_max, Some(None));
        assert_eq!(patch.quota_tpm, Some(Some(100)));
        assert!(patch.targets.is_none());

        let empty = serde_json::from_value(json!({})).expect("empty");
        assert!(validate_publication_update(empty).is_err());
    }

    #[test]
    fn client_model_detail_serialization_privacy() {
        let detail = ClientModelDetail {
            model: ClientModel {
                id: "m".to_string(),
                object: "model",
                created: 1,
                owned_by: "gateway",
                display_name: "M".to_string(),
                upstream_model: Some("m-upstream".to_string()),
                capabilities: vec!["chat".to_string()],
                context_window: Some(1000),
                max_output: Some(100),
                protocols: vec!["openai_chat".to_string()],
            },
            parameters: json!({"temperature": true}),
            catalog_version: "cat:7".to_string(),
        };
        let serialized = serde_json::to_string(&detail).unwrap();
        for forbidden in ["canonical", "price", "provider", "account", "allowed"] {
            assert!(
                !serialized.contains(forbidden),
                "client detail leaked {forbidden}: {serialized}"
            );
        }
        assert!(serialized.contains("upstream_model"));
        assert!(serialized.contains("catalog_version"));
        assert!(serialized.contains("parameters"));
    }
}

// ===========================================================================
// 工具默认模型（Tool Model Default）DTO / 校验（迭代 4）
// ===========================================================================

pub const ALL_SCOPE_TYPES: [&str; 3] = ["global", "user_group", "user"];
/// v0.1.x 可写的作用域（user_group 预留）。
pub const MAX_TOOL_SWITCH_MODELS: usize = 50;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefaultListQuery {
    #[serde(default)]
    pub tool_key: Option<String>,
    #[serde(default)]
    pub scope_type: Option<String>,
    #[serde(default)]
    pub scope_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefaultCreateRequest {
    pub tool_key: String,
    pub scope_type: String,
    #[serde(default)]
    pub scope_id: Option<Uuid>,
    pub default_model_group_id: Uuid,
    #[serde(default)]
    pub allow_switch: Option<bool>,
    #[serde(default)]
    pub allowed_model_group_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefaultUpdateRequest {
    #[serde(default)]
    pub default_model_group_id: Option<Uuid>,
    #[serde(default)]
    pub allow_switch: Option<bool>,
    #[serde(default)]
    pub allowed_model_group_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Clone)]
pub struct ToolDefaultFields {
    pub tool_key: String,
    pub scope_type: String,
    pub scope_id: Option<Uuid>,
    pub default_model_group_id: Uuid,
    pub allow_switch: bool,
    pub allowed_model_group_ids: Vec<Uuid>,
}

#[derive(Debug, Default, Clone)]
pub struct ToolDefaultPatch {
    pub default_model_group_id: Option<Uuid>,
    pub allow_switch: Option<bool>,
    /// 出现即全量替换。
    pub allowed_model_group_ids: Option<Vec<Uuid>>,
}

impl ToolDefaultPatch {
    pub fn is_empty(&self) -> bool {
        self.default_model_group_id.is_none()
            && self.allow_switch.is_none()
            && self.allowed_model_group_ids.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelGroupRef {
    pub id: Uuid,
    pub client_model_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolModelDefaultAdmin {
    pub id: Uuid,
    pub tool_key: String,
    pub scope_type: String,
    pub scope_id: Option<Uuid>,
    pub scope_label: Option<String>,
    pub default_model_group_id: Uuid,
    pub default_model_client_name: String,
    pub allow_switch: bool,
    pub allowed_model_groups: Vec<ModelGroupRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 内部解析结果（客户端默认模型端点/员工预览后续复用）。
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct ToolDefaultView {
    pub tool_key: String,
    /// 命中的作用域层级（global/user）。
    pub scope_type: String,
    /// 无有效默认（组未激活/未发布/仅可见不可用）时为 None。
    pub default_model_group_id: Option<Uuid>,
    pub default_model_client_name: Option<String>,
    pub allow_switch: bool,
    pub switchable_model_groups: Vec<ModelGroupRef>,
    pub warnings: Vec<String>,
}

fn validate_tool_key(raw: &str) -> Result<String, String> {
    let key = raw.trim();
    if key.is_empty() || key.len() > 64 {
        return Err("tool_key 必须为 1–64 个字符".to_string());
    }
    let valid = key.as_bytes()[0].is_ascii_lowercase()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    if !valid {
        return Err("tool_key 仅允许小写字母/数字/下划线，且以字母开头".to_string());
    }
    Ok(key.to_string())
}

fn validate_scope(
    scope_type: &str,
    scope_id: Option<Uuid>,
) -> Result<(String, Option<Uuid>), AppError> {
    let scope_type = scope_type.trim();
    if !ALL_SCOPE_TYPES.contains(&scope_type) {
        return Err(AppError::validation_field(
            "scope_type",
            format!("scope_type 必须是 {} 之一", ALL_SCOPE_TYPES.join("/")),
        ));
    }
    match scope_type {
        "global" => {
            if scope_id.is_some() {
                return Err(AppError::validation_field(
                    "scope_id",
                    "global 作用域的 scope_id 必须为空",
                ));
            }
            Ok((scope_type.to_string(), None))
        }
        "user" | "user_group" => {
            let Some(scope_id) = scope_id else {
                return Err(AppError::validation_field(
                    "scope_id",
                    format!("{scope_type} 作用域必须提供 scope_id"),
                ));
            };
            Ok((scope_type.to_string(), Some(scope_id)))
        }
        _ => unreachable!("scope_type validated against ALL_SCOPE_TYPES"),
    }
}

fn validate_switch_whitelist(raw: Option<Vec<Uuid>>) -> Result<Vec<Uuid>, AppError> {
    let Some(values) = raw else {
        return Ok(Vec::new());
    };
    if values.len() > MAX_TOOL_SWITCH_MODELS {
        return Err(AppError::validation_field(
            "allowed_model_group_ids",
            format!("可切换白名单最多 {MAX_TOOL_SWITCH_MODELS} 个"),
        ));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        if seen.insert(value) {
            out.push(value);
        }
    }
    Ok(out)
}

pub fn validate_tool_default_create(
    request: ToolDefaultCreateRequest,
) -> Result<ToolDefaultFields, AppError> {
    let tool_key = validate_tool_key(&request.tool_key)
        .map_err(|message| AppError::validation_field("tool_key", message))?;
    let (scope_type, scope_id) = validate_scope(&request.scope_type, request.scope_id)?;
    let allowed_model_group_ids = validate_switch_whitelist(request.allowed_model_group_ids)?;
    Ok(ToolDefaultFields {
        tool_key,
        scope_type,
        scope_id,
        default_model_group_id: request.default_model_group_id,
        allow_switch: request.allow_switch.unwrap_or(false),
        allowed_model_group_ids,
    })
}

pub fn validate_tool_default_update(
    request: ToolDefaultUpdateRequest,
) -> Result<ToolDefaultPatch, AppError> {
    let patch = ToolDefaultPatch {
        default_model_group_id: request.default_model_group_id,
        allow_switch: request.allow_switch,
        allowed_model_group_ids: match request.allowed_model_group_ids {
            Some(values) => Some(validate_switch_whitelist(Some(values))?),
            None => None,
        },
    };
    if patch.is_empty() {
        return Err(AppError::validation_field(
            "body",
            "至少提供一个要更新的字段",
        ));
    }
    Ok(patch)
}

pub fn validate_tool_default_list_query(
    query: &ToolDefaultListQuery,
) -> Result<(Option<String>, Option<String>), AppError> {
    let tool_key = match query.tool_key.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(value) => Some(
            validate_tool_key(value)
                .map_err(|message| AppError::validation_field("tool_key", message))?,
        ),
    };
    let scope_type = match query.scope_type.as_deref() {
        None => None,
        Some(value) if ALL_SCOPE_TYPES.contains(&value) => Some(value.to_string()),
        Some(_) => {
            return Err(AppError::validation_field(
                "scope_type",
                format!("scope_type 必须是 {} 之一", ALL_SCOPE_TYPES.join("/")),
            ));
        }
    };
    Ok((tool_key, scope_type))
}

#[cfg(test)]
mod tool_default_tests {
    use serde_json::json;

    use super::*;

    fn create(value: serde_json::Value) -> ToolDefaultCreateRequest {
        serde_json::from_value(value).expect("create request")
    }

    #[test]
    fn tool_key_rules() {
        assert_eq!(validate_tool_key(" claude_code ").unwrap(), "claude_code");
        assert!(validate_tool_key("").is_err());
        assert!(validate_tool_key("Claude").is_err());
        assert!(validate_tool_key("1tool").is_err());
        assert!(validate_tool_key(&"a".repeat(65)).is_err());
    }

    #[test]
    fn scope_rules() {
        let global = validate_tool_default_create(create(json!({
            "tool_key": "codex",
            "scope_type": "global",
            "scope_id": null,
            "default_model_group_id": Uuid::new_v4()
        })))
        .unwrap();
        assert_eq!(global.scope_type, "global");
        assert!(global.scope_id.is_none());
        assert!(!global.allow_switch);

        let group_scope = validate_tool_default_create(create(json!({
            "tool_key": "codex",
            "scope_type": "user_group",
            "scope_id": Uuid::new_v4(),
            "default_model_group_id": Uuid::new_v4()
        })))
        .unwrap();
        assert_eq!(group_scope.scope_type, "user_group");
        assert!(group_scope.scope_id.is_some());
        assert!(
            validate_tool_default_create(create(json!({
                "tool_key": "codex",
                "scope_type": "user_group",
                "default_model_group_id": Uuid::new_v4()
            })))
            .is_err(),
            "user_group scope requires scope_id"
        );

        let user = validate_tool_default_create(create(json!({
            "tool_key": "codex",
            "scope_type": "user",
            "scope_id": Uuid::new_v4(),
            "default_model_group_id": Uuid::new_v4(),
            "allow_switch": true
        })))
        .unwrap();
        assert!(user.allow_switch && user.scope_id.is_some());

        assert!(
            validate_tool_default_create(create(json!({
                "tool_key": "codex",
                "scope_type": "global",
                "scope_id": Uuid::new_v4(),
                "default_model_group_id": Uuid::new_v4()
            })))
            .is_err()
        );
        assert!(
            validate_tool_default_create(create(json!({
                "tool_key": "codex",
                "scope_type": "user",
                "default_model_group_id": Uuid::new_v4()
            })))
            .is_err()
        );
    }

    #[test]
    fn whitelist_limit_and_dedup() {
        let ids: Vec<Uuid> = (0..=MAX_TOOL_SWITCH_MODELS)
            .map(|_| Uuid::new_v4())
            .collect();
        let too_many = validate_tool_default_create(create(json!({
            "tool_key": "codex",
            "scope_type": "global",
            "default_model_group_id": Uuid::new_v4(),
            "allowed_model_group_ids": ids
        })))
        .unwrap_err();
        assert!(matches!(too_many, AppError::Validation { .. }));

        let first = Uuid::new_v4();
        let dedup = validate_tool_default_create(create(json!({
            "tool_key": "codex",
            "scope_type": "global",
            "default_model_group_id": first,
            "allowed_model_group_ids": [first, first, Uuid::new_v4()]
        })))
        .unwrap();
        assert_eq!(dedup.allowed_model_group_ids.len(), 2);
    }

    #[test]
    fn update_patch_semantics() {
        let empty = validate_tool_default_update(serde_json::from_value(json!({})).expect("empty"))
            .unwrap_err();
        assert!(matches!(empty, AppError::Validation { .. }));

        let patch = validate_tool_default_update(
            serde_json::from_value(json!({"allow_switch": false, "allowed_model_group_ids": []}))
                .expect("patch"),
        )
        .unwrap();
        assert_eq!(patch.allow_switch, Some(false));
        assert_eq!(patch.allowed_model_group_ids, Some(Vec::new()));
        assert!(patch.default_model_group_id.is_none());
    }

    #[test]
    fn list_query_validation() {
        let query = ToolDefaultListQuery {
            tool_key: Some(" claude_code ".to_string()),
            scope_type: Some("user".to_string()),
            scope_id: None,
            limit: None,
            offset: None,
        };
        let (tool_key, scope_type) = validate_tool_default_list_query(&query).unwrap();
        assert_eq!(tool_key.as_deref(), Some("claude_code"));
        assert_eq!(scope_type.as_deref(), Some("user"));

        let bad = ToolDefaultListQuery {
            scope_type: Some("team".to_string()),
            ..ToolDefaultListQuery::default()
        };
        assert!(validate_tool_default_list_query(&bad).is_err());
    }

    #[test]
    fn tool_default_view_serialization_privacy() {
        let view = ToolDefaultView {
            tool_key: "codex".to_string(),
            scope_type: "global".to_string(),
            default_model_group_id: Some(Uuid::new_v4()),
            default_model_client_name: Some("g-one".to_string()),
            allow_switch: true,
            switchable_model_groups: vec![ModelGroupRef {
                id: Uuid::new_v4(),
                client_model_name: "g-one".to_string(),
            }],
            warnings: Vec::new(),
        };
        let serialized = serde_json::to_string(&view).unwrap();
        for forbidden in ["canonical", "price", "provider", "account", "allowed"] {
            assert!(
                !serialized.contains(forbidden),
                "tool default view leaked {forbidden}: {serialized}"
            );
        }
    }
}

// ===========================================================================
// 员工目录预览（Admin catalog preview，迭代 5）
// ===========================================================================

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogPreviewQuery {
    pub user_id: Uuid,
    #[serde(default)]
    pub device_id: Option<Uuid>,
    #[serde(default)]
    pub key_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewSubject {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub user_status: String,
    pub device_id: Option<Uuid>,
    pub device_name: Option<String>,
    pub key_id: Option<Uuid>,
    pub key_allowed_models: Vec<String>,
    /// v0.1.x 恒空（access 用户组预留）。
    pub user_group_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminPreviewModel {
    #[serde(flatten)]
    pub model: ClientModel,
    pub visible: bool,
    pub allowed: bool,
    pub matched_by: Option<String>,
    pub publication_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminPreviewToolDefault {
    pub tool_key: String,
    pub scope_type: String,
    /// 命中的层级（global/user）；由 `ToolDefaultView.scope_type` 映射。
    pub source: String,
    pub default_model: Option<String>,
    pub switchable_models: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminCatalogPreview {
    pub catalog_version: String,
    pub subject: PreviewSubject,
    pub models: Vec<AdminPreviewModel>,
    pub tool_defaults: Vec<AdminPreviewToolDefault>,
    pub enforcement: Enforcement,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod preview_tests {
    use serde_json::json;

    use super::*;

    fn subject(user_id: Uuid, whitelist: &[&str]) -> CatalogSubject {
        CatalogSubject {
            user_id,
            device_id: None,
            user_group_ids: Vec::new(),
            key_allowed_models: whitelist.iter().map(|item| item.to_string()).collect(),
        }
    }

    fn target(
        subject_type: &str,
        subject_id: Option<Uuid>,
        visible: bool,
        allowed: bool,
    ) -> TargetView {
        TargetView {
            subject_type: subject_type.to_string(),
            subject_id,
            visible,
            allowed,
        }
    }

    #[test]
    fn matched_by_reports_user_tier() {
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        let targets = vec![
            target("all_users", None, true, true),
            target("user", Some(alice), true, false),
        ];
        let (alice_eff, matched) =
            effective_publication_with_match(&targets, &subject(alice, &[]), "m").unwrap();
        assert_eq!(matched, "user");
        assert!(alice_eff.visible && !alice_eff.allowed);

        let (bob_eff, matched) =
            effective_publication_with_match(&targets, &subject(bob, &[]), "m").unwrap();
        assert_eq!(matched, "all_users");
        assert!(bob_eff.visible && bob_eff.allowed);
    }

    #[test]
    fn matched_by_fails_closed_for_reserved_types() {
        let alice = Uuid::new_v4();
        let targets = vec![
            target("device", Some(Uuid::new_v4()), true, true),
            target("user_group", Some(Uuid::new_v4()), true, true),
        ];
        assert!(effective_publication_with_match(&targets, &subject(alice, &[]), "m").is_none());
    }

    #[test]
    fn preview_model_serialization_privacy() {
        let model = AdminPreviewModel {
            model: ClientModel {
                id: "m-one".to_string(),
                object: "model",
                created: 1,
                owned_by: "gateway",
                display_name: "M One".to_string(),
                upstream_model: Some("m1".to_string()),
                capabilities: vec!["chat".to_string()],
                context_window: Some(1000),
                max_output: Some(100),
                protocols: vec!["openai_chat".to_string()],
            },
            visible: true,
            allowed: false,
            matched_by: Some("user".to_string()),
            publication_id: Some(Uuid::new_v4()),
        };
        let serialized = serde_json::to_string(&model).unwrap();
        for forbidden in ["canonical", "price", "provider", "account"] {
            assert!(
                !serialized.contains(forbidden),
                "preview model leaked {forbidden}: {serialized}"
            );
        }
        for expected in [
            "visible",
            "allowed",
            "matched_by",
            "publication_id",
            "upstream_model",
        ] {
            assert!(
                serialized.contains(expected),
                "missing {expected}: {serialized}"
            );
        }
    }

    #[test]
    fn preview_dto_shape() {
        let preview = AdminCatalogPreview {
            catalog_version: "cat:7".to_string(),
            subject: PreviewSubject {
                user_id: Uuid::new_v4(),
                email: "alice@corp.example.com".to_string(),
                display_name: "Alice".to_string(),
                user_status: "active".to_string(),
                device_id: None,
                device_name: None,
                key_id: None,
                key_allowed_models: vec!["m-one".to_string()],
                user_group_ids: Vec::new(),
            },
            models: Vec::new(),
            tool_defaults: vec![AdminPreviewToolDefault {
                tool_key: "codex".to_string(),
                scope_type: "global".to_string(),
                source: "global".to_string(),
                default_model: Some("m-one".to_string()),
                switchable_models: Vec::new(),
                warnings: Vec::new(),
            }],
            enforcement: Enforcement::default(),
            warnings: vec!["no_visible_models".to_string()],
        };
        let serialized = serde_json::to_string(&preview).unwrap();
        for forbidden in ["canonical", "price", "provider", "account"] {
            assert!(
                !serialized.contains(forbidden),
                "preview leaked {forbidden}: {serialized}"
            );
        }
        assert!(serialized.contains("enforced") && serialized.contains("config_only"));
        assert!(serialized.contains("user_group_ids"));
    }

    #[test]
    fn preview_query_requires_user_id() {
        let parsed: CatalogPreviewQuery = serde_json::from_value(json!({
            "user_id": Uuid::new_v4(),
            "device_id": null
        }))
        .expect("query");
        assert!(parsed.device_id.is_none());
        assert!(serde_json::from_value::<CatalogPreviewQuery>(json!({})).is_err());
    }
}

// ===========================================================================
// 聚合建议（Merge Suggestions，补全）+ 客户端默认模型 DTO
// ===========================================================================

pub const MAX_SUGGESTIONS: usize = 200;
pub const MAX_RELATED_GROUPS: usize = 20;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestionListQuery {
    #[serde(default)]
    pub canonical: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmMergeSuggestionRequest {
    pub action: String,
    #[serde(default)]
    pub client_model_name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub target_group_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectMergeSuggestionRequest {
    #[serde(default)]
    pub note: Option<String>,
}

/// 校验后的 confirm 动作。
#[derive(Debug, Clone)]
pub enum MergeConfirmAction {
    CreateGroup {
        client_model_name: String,
        display_name: String,
    },
    MergeIntoGroup {
        target_group_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct SuggestionDeployment {
    pub id: Uuid,
    pub upstream_model_id: Uuid,
    pub upstream_model: String,
    pub canonical: String,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_kind: String,
    pub provider_account_id: Uuid,
    pub account_name: String,
    pub account_region: String,
    pub priority: i32,
    pub weight: i32,
    pub enabled: bool,
    pub price_input: Option<f64>,
    pub price_output: Option<f64>,
    pub currency: String,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedGroup {
    pub id: Uuid,
    pub client_model_name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffValue {
    pub deployment_id: Uuid,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffItem {
    pub field: String,
    pub risk_flag: String,
    pub values: Vec<DiffValue>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeSuggestion {
    pub suggestion_key: String,
    pub canonical: String,
    pub upstream_model_name: String,
    pub recommendation: &'static str,
    pub deployment_count: i64,
    pub provider_count: i64,
    pub deployments: Vec<SuggestionDeployment>,
    pub related_groups: Vec<RelatedGroup>,
    pub diff: Vec<DiffItem>,
    pub risk_flags: Vec<String>,
    pub first_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeConfirmation {
    pub suggestion_key: String,
    pub decision: &'static str,
    pub group: ModelGroupAdmin,
    pub moved_deployment_ids: Vec<Uuid>,
    pub decided_at: DateTime<Utc>,
    pub decided_by: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeRejection {
    pub suggestion_key: String,
    pub decision: &'static str,
    pub note: String,
    pub decided_at: DateTime<Utc>,
    pub decided_by: Option<Uuid>,
}

/// 建议指纹：`sha256(canonical \n sorted(names) \n sorted(deployment_ids))[:32]`。
pub fn suggestion_key(
    canonical: &str,
    upstream_model_names: &[String],
    deployment_ids: &[Uuid],
) -> String {
    let mut names = upstream_model_names.to_vec();
    names.sort();
    let mut ids: Vec<String> = deployment_ids.iter().map(ToString::to_string).collect();
    ids.sort();
    let payload = format!("{}\n{}\n{}", canonical, names.join("\n"), ids.join("\n"));
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(payload.as_bytes());
    hex::encode(&digest[..16])
}

pub fn validate_confirm_suggestion_request(
    request: ConfirmMergeSuggestionRequest,
) -> Result<MergeConfirmAction, AppError> {
    let action = request.action.trim();
    match action {
        "create_group" => {
            if request.target_group_id.is_some() {
                return Err(AppError::validation_field(
                    "target_group_id",
                    "create_group 不能指定 target_group_id",
                ));
            }
            let Some(raw_name) = request.client_model_name.as_deref() else {
                return Err(AppError::validation_field(
                    "client_model_name",
                    "create_group 必须提供 client_model_name",
                ));
            };
            let client_model_name = validate_client_model_name(raw_name)
                .map_err(|message| AppError::validation_field("client_model_name", message))?;
            let display_name = validate_display_name(request.display_name.as_deref())
                .map_err(|message| AppError::validation_field("display_name", message))?;
            Ok(MergeConfirmAction::CreateGroup {
                client_model_name,
                display_name,
            })
        }
        "merge_into_group" => {
            if request.client_model_name.is_some() || request.display_name.is_some() {
                return Err(AppError::validation_field(
                    "client_model_name",
                    "merge_into_group 不能指定 client_model_name/display_name",
                ));
            }
            let Some(target_group_id) = request.target_group_id else {
                return Err(AppError::validation_field(
                    "target_group_id",
                    "merge_into_group 必须提供 target_group_id",
                ));
            };
            Ok(MergeConfirmAction::MergeIntoGroup { target_group_id })
        }
        _ => Err(AppError::validation_field(
            "action",
            "action 仅支持 create_group/merge_into_group",
        )),
    }
}

pub fn validate_reject_suggestion_request(
    request: RejectMergeSuggestionRequest,
) -> Result<String, AppError> {
    validate_optional_text(request.note.as_deref(), "note", 500)
        .map_err(|message| AppError::validation_field("note", message))
}

// ---------- 客户端每工具默认模型（GET /v1/model-defaults） ----------

#[derive(Debug, Clone, Serialize)]
pub struct ClientModelDefault {
    pub tool_key: String,
    pub default_model: String,
    pub allow_switch: bool,
    pub switchable_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientModelDefaultsList {
    pub object: &'static str,
    pub data: Vec<ClientModelDefault>,
    pub catalog_version: String,
}

#[cfg(test)]
mod merge_suggestion_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn suggestion_key_is_stable_and_order_insensitive() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let names = vec!["m1".to_string(), "m2".to_string()];
        let key_one = suggestion_key("c1", &names, &[id_a, id_b]);
        let key_two = suggestion_key("c1", &["m2".to_string(), "m1".to_string()], &[id_b, id_a]);
        assert_eq!(key_one, key_two);
        assert_eq!(key_one.len(), 32, "{key_one}");
        assert!(key_one.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_ne!(key_one, suggestion_key("c2", &names, &[id_a, id_b]));
        assert_ne!(key_one, suggestion_key("c1", &names, &[id_a]));
        assert_ne!(
            key_one,
            suggestion_key("c1", &["m1".to_string()], &[id_a, id_b])
        );
    }

    #[test]
    fn confirm_request_validation() {
        let action = validate_confirm_suggestion_request(
            serde_json::from_value(json!({
                "action": "create_group",
                "client_model_name": "agg-one",
                "display_name": "Agg One"
            }))
            .expect("create"),
        )
        .unwrap();
        match action {
            MergeConfirmAction::CreateGroup {
                client_model_name,
                display_name,
            } => {
                assert_eq!(client_model_name, "agg-one");
                assert_eq!(display_name, "Agg One");
            }
            other => panic!("unexpected action {other:?}"),
        }
        assert!(
            validate_confirm_suggestion_request(
                serde_json::from_value(json!({"action": "create_group"})).expect("missing")
            )
            .is_err()
        );
        assert!(
            validate_confirm_suggestion_request(
                serde_json::from_value(json!({
                    "action": "create_group",
                    "client_model_name": "agg-one",
                    "target_group_id": Uuid::new_v4()
                }))
                .expect("bad create")
            )
            .is_err()
        );
        let target = Uuid::new_v4();
        let action = validate_confirm_suggestion_request(
            serde_json::from_value(json!({
                "action": "merge_into_group",
                "target_group_id": target
            }))
            .expect("merge"),
        )
        .unwrap();
        assert!(matches!(
            action,
            MergeConfirmAction::MergeIntoGroup { target_group_id } if target_group_id == target
        ));
        assert!(
            validate_confirm_suggestion_request(
                serde_json::from_value(json!({"action": "merge_into_group"}))
                    .expect("missing target")
            )
            .is_err()
        );
        assert!(
            validate_confirm_suggestion_request(
                serde_json::from_value(json!({"action": "merge_all"})).expect("bad action")
            )
            .is_err()
        );
    }

    #[test]
    fn reject_request_validation() {
        let note = validate_reject_suggestion_request(
            serde_json::from_value(json!({"note": "版本不同"})).expect("reject"),
        )
        .unwrap();
        assert_eq!(note, "版本不同");
        let note = validate_reject_suggestion_request(
            serde_json::from_value(json!({})).expect("empty reject"),
        )
        .unwrap();
        assert!(note.is_empty());
        assert!(
            validate_reject_suggestion_request(
                serde_json::from_value(json!({"note": "x".repeat(501)})).expect("long")
            )
            .is_err()
        );
    }

    #[test]
    fn client_model_defaults_serialization() {
        let list = ClientModelDefaultsList {
            object: "list",
            data: vec![ClientModelDefault {
                tool_key: "codex".to_string(),
                default_model: "g-one".to_string(),
                allow_switch: true,
                switchable_models: vec!["g-one".to_string(), "g-two".to_string()],
            }],
            catalog_version: "cat:7".to_string(),
        };
        let serialized = serde_json::to_string(&list).unwrap();
        for forbidden in [
            "scope_id",
            "whitelist",
            "canonical",
            "price",
            "provider",
            "account",
            "warnings",
            "allowed",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "client model defaults leaked {forbidden}: {serialized}"
            );
        }
        assert!(serialized.contains("catalog_version"));
    }
}
