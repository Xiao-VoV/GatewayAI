//! metering 领域类型：用法状态/来源、路由来源与 `UsageEvent`（proxy → metering 进程内契约）。
//!
//! 字段/语义以 `设计文档（暂未开源）` §2.2/§2.3 与
//! `设计文档（暂未开源）` §8.2 为准；事件为纯数据（无正文/凭证）。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

/// 计费/台账币种：S4 固定 CNY（`usage_records.currency` DB CHECK）。
pub const CURRENCY_CNY: &str = "CNY";
/// 价目币种：USD 价目由 metering 按 `GW_USD_CNY_RATE` 折算并写 fx 快照。
pub const PRICE_CURRENCY_CNY: &str = "CNY";
pub const PRICE_CURRENCY_USD: &str = "USD";

/// 用量状态（六值，与 `usage_records_status_check` 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageStatus {
    /// 上游 2xx 且响应/流完整。
    Success,
    /// 上游非 2xx / 连接失败。
    UpstreamError,
    /// 上游超时。
    Timeout,
    /// 网关内部错误（无上游计费）。
    GatewayError,
    /// 客户端在流式过程中断开。
    ClientDisconnect,
    /// 已鉴权但被网关拒绝（429/403/404/413/400）。
    Rejected,
}

impl UsageStatus {
    /// 落库/日志用的小写字符串（DB CHECK 六值）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::UpstreamError => "upstream_error",
            Self::Timeout => "timeout",
            Self::GatewayError => "gateway_error",
            Self::ClientDisconnect => "client_disconnect",
            Self::Rejected => "rejected",
        }
    }
}

/// token 来源（与 `usage_records_usage_source_check` 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    /// 上游显式返回 usage。
    Upstream,
    /// 网关按字符粗估（`cost::estimate_tokens`）。
    Estimated,
    /// 无 usage（拒绝/未调用上游）。
    Missing,
}

impl UsageSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upstream => "upstream",
            Self::Estimated => "estimated",
            Self::Missing => "missing",
        }
    }
}

/// 路由解析来源（与 `usage_records_source_check` 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    /// S4：routing.model_aliases。
    Alias,
    /// v0.1.x：catalog 模型组/部署。
    #[allow(dead_code)] // catalog v0.1.x 接入后移除
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

/// 一次已鉴权 `/v1` 推理请求的用量事件（proxy → metering）。
///
/// 401/无身份请求不写 usage（只写 observability 的 `request_logs`）；
/// `cost` 由 metering 计算，proxy 只传 token 与价目快照。
#[derive(Debug, Clone)]
pub struct UsageEvent {
    /// platform `x-request-id`；幂等键（`uq_usage_records_request_id`）。
    pub request_id: String,
    /// 请求开始时间（proxy handler 进入时间，UTC）。
    pub created_at: DateTime<Utc>,
    /// access `AuthContext` 身份（401 不写 usage，故必填）。
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub key_id: Uuid,
    /// 客户端请求的对外模型名；`model_not_found` 时为原始请求串。
    pub model_alias: String,
    /// 实际/首选上游模型；未解析成功为 None。
    pub resolved_upstream_model: Option<String>,
    /// 供应商 id 快照（内部账务/排障，不返回客户端）。
    pub provider_id: Option<Uuid>,
    /// 供应商名快照（删除后仍可读）。
    pub provider_name: Option<String>,
    /// catalog v0.1.x 部署归属；S4 为 None（soft reference）。
    pub deployment_id: Option<Uuid>,
    /// 解析来源；S4 = `RouteSource::Alias`。
    pub source: RouteSource,
    pub is_stream: bool,
    /// 最终 attempt 候选序号 > 0（同候选换账号重试不算 fallback）。
    pub fallback: bool,
    /// 实际上游 HTTP 尝试次数（S4 1..=2，DB 约束 1..=5）。
    pub attempt_count: i32,
    /// 输入 token；None 视为 0（拒绝/无 usage）。
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    /// token 来源（upstream/estimated/missing）。
    pub usage_source: UsageSource,
    /// 价目快照：每 1,000,000 tokens 的单价（CNY/USD，见 `price_currency`）。
    pub price_input: Option<Decimal>,
    pub price_output: Option<Decimal>,
    /// 价目币种；None 视为 CNY（无价目）。
    pub price_currency: Option<String>,
    /// 返回给客户端的 HTTP 状态码。
    pub http_status: i32,
    /// 台账状态（六值）。
    pub status: UsageStatus,
    /// problem `code`（≤64，如 `budget_exceeded`）；不存上游响应体。
    pub error_code: Option<String>,
    /// 端到端耗时（从 handler 进入到终结）。
    pub latency_ms: i64,
    /// 首 token 耗时（仅流式；非流式由 proxy 置 None）。
    pub ttft_ms: Option<i64>,
    /// 成本币种：恒 `CNY`（写库固定 CNY；透传仅为契约完整性）。
    pub currency: String,
}

// ---------- S5 管理查询 DTO ----------

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// 指标集合（`/overview` 的 today/trend/totals，`/usage` 的 totals/items 共用）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct Metrics {
    pub requests: i64,
    pub success_requests: i64,
    pub error_requests: i64,
    pub rejected_requests: i64,
    pub client_disconnect_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: String,
    /// 仅 overview 出现；usage items/totals 不输出。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TodayMetrics {
    pub day: NaiveDate,
    #[serde(flatten)]
    pub metrics: Metrics,
    pub status_breakdown: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendDay {
    pub day: NaiveDate,
    #[serde(flatten)]
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverviewResponse {
    pub currency: &'static str,
    pub timezone: &'static str,
    pub generated_at: DateTime<Utc>,
    pub days: i64,
    pub today: TodayMetrics,
    pub trend: Vec<TrendDay>,
    pub totals: Metrics,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<Uuid>,
    #[serde(flatten)]
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageResponse {
    pub currency: &'static str,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub group_by: Vec<&'static str>,
    pub totals: Metrics,
    pub items: Vec<UsageItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverviewQuery {
    #[serde(default)]
    pub days: Option<i64>,
}

/// `GET /api/client/usage` 查询参数（days 1–30，默认 7）。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientUsageQuery {
    #[serde(default)]
    pub days: Option<i64>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageQuery {
    #[serde(default)]
    pub from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub to: Option<DateTime<Utc>>,
    #[serde(default)]
    pub group_by: Option<String>,
    #[serde(default)]
    pub user_id: Option<Uuid>,
    #[serde(default)]
    pub key_id: Option<Uuid>,
    #[serde(default)]
    pub provider_id: Option<Uuid>,
    #[serde(default)]
    pub model_alias: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// 请求详情合并的计量块（observability `/requests/{id}` 用；只读）。
#[derive(Debug, Clone, Serialize)]
pub struct UsageBlock {
    pub input_tokens: i32,
    pub output_tokens: i32,
    /// CNY，固定 6 位小数字符串。
    pub cost: String,
    pub currency: String,
    pub latency_ms: i32,
    pub ttft_ms: Option<i32>,
    pub is_stream: bool,
    pub fallback: bool,
    pub status: String,
    pub http_status: Option<i32>,
    pub error_code: Option<String>,
    pub usage_source: String,
}
