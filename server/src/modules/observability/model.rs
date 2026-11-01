//! observability 领域类型：协议枚举与 `RequestLogEvent`（数据面 handler → observability）。
//!
//! 字段/语义以 `设计文档（暂未开源）` §2.2 与
//! `设计文档（暂未开源）` §8.3 为准。
//! 事件是**封闭结构**（无 headers/body 字段），从类型上杜绝误传敏感数据。

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// 数据面协议维度（与 `request_logs_protocol_check` 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolKind {
    ChatCompletions,
    Messages,
    #[allow(dead_code)] // v0.1.x 多协议接入后移除（S4 仅 Chat/Messages）
    Responses,
    #[allow(dead_code)] // v0.1.x Gemini 原生协议接入后移除
    Gemini,
    /// 目录查询 `/v1/models`（catalog 写）。
    #[allow(dead_code)] // catalog `/v1/models` 日志接入后移除
    Models,
    #[allow(dead_code)] // v0.1.x
    Other,
}

impl ProtocolKind {
    /// 落库字符串（DB CHECK 六值）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Messages => "messages",
            Self::Responses => "responses",
            Self::Gemini => "gemini",
            Self::Models => "models",
            Self::Other => "other",
        }
    }
}

/// 一个到达数据面的请求的元数据日志事件（含 401/网关拒绝/上游失败）。
///
/// 由数据面 handler 在响应完成/流结束/出错后构造；写入前经 `sanitize` 二次防御。
/// `client_ip` 用 `Option<String>`：proxy 的 `RequestContext.client_ip`
/// 已是 `String`（peer IP / 受信 XFF），observability 写入前解析为 `IpAddr` 规范化。
#[derive(Debug, Clone)]
pub struct RequestLogEvent {
    /// platform `x-request-id`；幂等键（`uq_request_logs_request_id`）。
    pub request_id: String,
    /// S4 = request_id；v0.1.x W3C traceparent。
    pub trace_id: Option<String>,
    /// HTTP 方法（写入前统一大写）。
    pub method: String,
    /// 路由模板（如 `/v1/chat/completions`），不含 query string。
    pub path: String,
    pub protocol: ProtocolKind,
    /// 上游协议（v0.1.x：openai_chat/anthropic/…）。
    pub upstream_protocol: Option<String>,
    /// 适配器标识（proxy 注册表：chat_passthrough/messages_to_chat/…）。
    pub adapter: Option<String>,
    /// 401/无身份请求为 None。
    pub user_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
    pub key_id: Option<Uuid>,
    /// Key 前缀快照（如 `sk-gw-abcd`）；完整 Key 永不入库。
    pub key_prefix: Option<String>,
    /// 客户端请求的对外模型名。
    pub model_alias: Option<String>,
    /// 实际上游模型。
    pub upstream_model: Option<String>,
    /// 供应商 id 快照。
    pub provider_id: Option<Uuid>,
    /// 供应商名快照（删除后仍可读）。
    pub provider: Option<String>,
    pub is_stream: bool,
    /// 最终 attempt 候选序号 > 0。
    pub fallback: bool,
    /// 实际上游 HTTP 尝试次数（S4 1..=2，DB 约束 1..=5）。
    pub attempt_count: i32,
    /// 返回给客户端的 HTTP 状态码；proxy 内部未映射时为 0/None 由 sanitizer 归一。
    pub status_code: i32,
    /// 端到端耗时。
    pub latency_ms: i64,
    /// 首 token 耗时（仅流式）。
    pub ttft_ms: Option<i64>,
    /// 发送给客户端的字节数（流式为累计）。
    pub response_size: Option<i64>,
    /// problem code（≤64，snake_case）；不存上游响应体。
    pub error_code: Option<String>,
    /// peer IP（XFF 仅在 `GW_TRUST_PROXY_HEADERS=true` 时由 proxy 采信）。
    pub client_ip: Option<String>,
    /// UA（写入前截断 512、去控制字符）。
    pub user_agent: Option<String>,
    /// 请求开始时间（UTC）。
    pub created_at: DateTime<Utc>,
}

// ---------- S5 管理查询 DTO ----------

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct RequestLogItem {
    pub id: Uuid,
    pub request_id: String,
    pub trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub protocol: String,
    pub upstream_protocol: Option<String>,
    pub adapter: Option<String>,
    pub user_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
    pub key_id: Option<Uuid>,
    pub key_prefix: Option<String>,
    pub model_alias: Option<String>,
    pub upstream_model: Option<String>,
    pub provider: Option<String>,
    pub provider_id: Option<Uuid>,
    pub is_stream: bool,
    pub fallback: bool,
    pub attempt_count: i32,
    pub status_code: Option<i32>,
    pub latency_ms: i32,
    pub ttft_ms: Option<i32>,
    pub response_size: Option<i64>,
    pub error_code: Option<String>,
    pub client_ip: Option<String>,
    pub user_agent: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLogDetail {
    #[serde(flatten)]
    pub item: RequestLogItem,
    pub usage: Option<crate::modules::metering::model::UsageBlock>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLogPage {
    pub items: Vec<RequestLogItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestListQuery {
    #[serde(default)]
    pub from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub to: Option<DateTime<Utc>>,
    #[serde(default)]
    pub user_id: Option<Uuid>,
    #[serde(default)]
    pub device_id: Option<Uuid>,
    #[serde(default)]
    pub key_id: Option<Uuid>,
    #[serde(default)]
    pub key_prefix: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub status_class: Option<String>,
    #[serde(default)]
    pub status_code: Option<i32>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

// ---------- S5.1 审计 / CSV 导出 DTO ----------

use serde_json::Value;

/// 审计事件（管理写操作 → observability；只追加，明细写入前经 `sanitize_audit_detail`）。
///
/// 字段以 `设计文档（暂未开源）` §2 为准；
/// 事件是封闭结构（无 headers/body），从类型上杜绝误传敏感数据。
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// `admin` / `system` / `anonymous`（与 `audit_logs_actor_type_check` 一致）。
    pub actor_type: String,
    /// 管理员 id（匿名/系统事件为 None）。
    pub actor_id: Option<Uuid>,
    /// 操作者/尝试邮箱快照。
    pub actor_email: Option<String>,
    /// 动作注册表：`{domain}.{resource}.{verb}`，如 `access.key.revoked`。
    pub action: String,
    pub resource_type: String,
    /// 资源 id（uuid 或复合串，≤512）。
    pub resource_id: Option<String>,
    /// `success` / `failure` / `denied`（与 `audit_logs_outcome_check` 一致）。
    pub outcome: String,
    /// 只存 IP（无端口）。
    pub client_ip: Option<String>,
    /// UA（写入前截断 512、去控制字符）。
    pub user_agent: String,
    /// 关联管理台请求 id（platform x-request-id）。
    pub request_id: Option<String>,
    /// 脱敏前后值摘要（写入前 sanitize，序列化 ≤4KB）。
    pub detail: Value,
}

/// 审计列表行（= `audit_logs` 列；detail 已脱敏）。
#[derive(Debug, Clone, Serialize)]
pub struct AuditItem {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub actor_type: String,
    pub actor_id: Option<Uuid>,
    pub actor_email: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub outcome: String,
    pub client_ip: Option<String>,
    pub user_agent: String,
    pub request_id: Option<String>,
    pub detail: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditPage {
    pub items: Vec<AuditItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// `GET /api/admin/audit` 查询参数（默认最近 7 天、最新 100 条、limit ≤500）。
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditListQuery {
    #[serde(default)]
    pub from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub to: Option<DateTime<Utc>>,
    #[serde(default)]
    pub actor_id: Option<Uuid>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub resource_type: Option<String>,
    #[serde(default)]
    pub resource_id: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}
