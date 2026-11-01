//! 请求终结记账：构造 metering `UsageEvent` / observability `RequestLogEvent` 并 fire-and-forget。
//!
//! 设计：proxy 技术设计 §8。规则：
//! - 每个已鉴权请求一条 usage（401/无身份只写 request_log）；
//! - 写事件不阻塞响应；DB 写失败由 metering/observability 内部重试 + warn；
//! - 事件是封闭结构：无 headers/body/凭证字段。

use std::time::Instant;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::modules::access::model::AuthContext;
use crate::modules::metering::model::{
    RouteSource as MeteringRouteSource, UsageEvent, UsageSource, UsageStatus,
};
use crate::modules::metering::service::record_usage_detached;
use crate::modules::observability::model::{ProtocolKind, RequestLogEvent};
use crate::modules::observability::service::record_request_log_detached;
use crate::platform::state::AppState;

use super::adapters::FrontendProtocol;
use super::route::PlannedTarget;
use super::settings::ProxySettings;

/// 请求终结时的结果快照。
#[derive(Debug, Clone)]
pub struct Outcome {
    pub status: UsageStatus,
    pub http_status: u16,
    pub error_code: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub usage_source: UsageSource,
    pub ttft_ms: Option<i64>,
    pub response_size: Option<i64>,
}

/// 一次数据面请求的终结上下文（handler 进入时创建，响应/流结束时 finalize）。
#[derive(Clone)]
pub struct Telemetry {
    pub state: AppState,
    pub request_id: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Instant,
    pub method: String,
    pub path: String,
    pub frontend: FrontendProtocol,
    pub adapter: Option<&'static str>,
    pub auth: Option<AuthContext>,
    pub model_alias: Option<String>,
    pub resolved: Option<ResolvedTarget>,
    pub is_stream: bool,
    pub fallback: bool,
    pub attempt_count: i32,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    /// catalog 来源的对外模型组名/版本（用于头/日志；S4 alias 为 None）。
    pub model_group: Option<String>,
    pub catalog_version: Option<String>,
    /// false = 该请求不写 usage（如 Key RPM 429：已入请求日志但不计账）。
    pub write_usage: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub upstream_model: String,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub deployment_id: Option<Uuid>,
    pub source: MeteringRouteSource,
    pub price_input: Option<rust_decimal::Decimal>,
    pub price_output: Option<rust_decimal::Decimal>,
    pub price_currency: Option<String>,
    pub model_group_id: Option<Uuid>,
}

impl Telemetry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: AppState,
        request_id: String,
        frontend: FrontendProtocol,
        method: &str,
        path: &str,
        client_ip: Option<String>,
        user_agent: Option<String>,
        is_stream: bool,
    ) -> Self {
        Self {
            state,
            request_id,
            created_at: Utc::now(),
            started_at: Instant::now(),
            method: method.to_string(),
            path: path.to_string(),
            frontend,
            adapter: None,
            auth: None,
            model_alias: None,
            resolved: None,
            is_stream,
            fallback: false,
            attempt_count: 0,
            client_ip,
            user_agent,
            model_group: None,
            catalog_version: None,
            write_usage: true,
        }
    }

    pub fn with_auth(&mut self, auth: AuthContext) {
        self.auth = Some(auth);
    }

    pub fn with_model_alias(&mut self, alias: String) {
        self.model_alias = Some(alias);
    }

    pub fn with_adapter(&mut self, adapter: &'static str) {
        self.adapter = Some(adapter);
    }

    pub fn with_target(&mut self, target: &PlannedTarget) {
        let source = match target.source {
            super::route::RouteSource::Alias => MeteringRouteSource::Alias,
            super::route::RouteSource::Catalog => MeteringRouteSource::Catalog,
        };
        self.resolved = Some(ResolvedTarget {
            upstream_model: target.upstream_model.clone(),
            provider_id: target.provider_id,
            provider_name: target.provider_name.clone(),
            deployment_id: target.deployment_id,
            source,
            price_input: target.price_input,
            price_output: target.price_output,
            price_currency: Some(target.currency.clone()),
            model_group_id: target.model_group_id,
        });
    }

    pub fn latency_ms(&self) -> i64 {
        i64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(i64::MAX)
    }

    /// 终结：写 usage（仅已鉴权）与 request_log（全部）；失败不影响主响应。
    pub fn finalize(self, outcome: Outcome) {
        let latency_ms = self.latency_ms();
        tracing::debug!(
            request_id = %self.request_id,
            source = self
                .resolved
                .as_ref()
                .map(|resolved| resolved.source.as_str())
                .unwrap_or("none"),
            deployment_id = ?self
                .resolved
                .as_ref()
                .and_then(|resolved| resolved.deployment_id),
            model_group_id = ?self
                .resolved
                .as_ref()
                .and_then(|resolved| resolved.model_group_id),
            model_group = self.model_group.as_deref().unwrap_or("none"),
            catalog_version = self.catalog_version.as_deref().unwrap_or("none"),
            http_status = outcome.http_status,
            status = outcome.status.as_str(),
            latency_ms,
            "proxy request finalized"
        );
        if self.write_usage
            && let Some(auth) = &self.auth
        {
            let resolved = self.resolved.as_ref();
            let event = UsageEvent {
                request_id: self.request_id.clone(),
                created_at: self.created_at,
                user_id: auth.user_id,
                device_id: auth.device_id,
                key_id: auth.key_id,
                model_alias: self.model_alias.clone().unwrap_or_default(),
                resolved_upstream_model: resolved.map(|resolved| resolved.upstream_model.clone()),
                provider_id: resolved.map(|resolved| resolved.provider_id),
                provider_name: resolved.map(|resolved| resolved.provider_name.clone()),
                deployment_id: resolved.and_then(|resolved| resolved.deployment_id),
                source: resolved
                    .map(|resolved| resolved.source)
                    .unwrap_or(MeteringRouteSource::Alias),
                is_stream: self.is_stream,
                fallback: self.fallback,
                attempt_count: self.attempt_count.max(1),
                input_tokens: outcome.input_tokens,
                output_tokens: outcome.output_tokens,
                usage_source: outcome.usage_source,
                price_input: resolved.and_then(|resolved| resolved.price_input),
                price_output: resolved.and_then(|resolved| resolved.price_output),
                price_currency: resolved.and_then(|resolved| resolved.price_currency.clone()),
                http_status: i32::from(outcome.http_status),
                status: outcome.status,
                error_code: outcome.error_code.clone(),
                latency_ms,
                ttft_ms: outcome.ttft_ms,
                currency: "CNY".to_string(),
            };
            record_usage_detached(&self.state, event);
        }

        let protocol = match self.frontend {
            FrontendProtocol::ChatCompletions => ProtocolKind::ChatCompletions,
            FrontendProtocol::Messages => ProtocolKind::Messages,
            FrontendProtocol::Responses => ProtocolKind::Responses,
            FrontendProtocol::Gemini => ProtocolKind::Gemini,
        };
        let upstream_protocol = match self.frontend {
            FrontendProtocol::Responses | FrontendProtocol::Gemini => {
                Some("chat_completions".to_string())
            }
            FrontendProtocol::ChatCompletions | FrontendProtocol::Messages => {
                Some("openai_chat".to_string())
            }
        };
        let resolved = self.resolved.as_ref();
        let event = RequestLogEvent {
            request_id: self.request_id.clone(),
            trace_id: Some(self.request_id.clone()),
            method: self.method.clone(),
            path: self.path.clone(),
            protocol,
            upstream_protocol,
            adapter: self.adapter.map(ToString::to_string),
            user_id: self.auth.as_ref().map(|auth| auth.user_id),
            device_id: self.auth.as_ref().map(|auth| auth.device_id),
            key_id: self.auth.as_ref().map(|auth| auth.key_id),
            key_prefix: self.auth.as_ref().and_then(|auth| auth.key_prefix.clone()),
            model_alias: self.model_alias.clone(),
            upstream_model: resolved.map(|resolved| resolved.upstream_model.clone()),
            provider_id: resolved.map(|resolved| resolved.provider_id),
            provider: resolved.map(|resolved| resolved.provider_name.clone()),
            is_stream: self.is_stream,
            fallback: self.fallback,
            attempt_count: self.attempt_count.max(1),
            status_code: i32::from(outcome.http_status),
            latency_ms,
            ttft_ms: outcome.ttft_ms,
            response_size: outcome.response_size,
            error_code: outcome.error_code,
            client_ip: self.client_ip.clone(),
            user_agent: self.user_agent.clone(),
            created_at: self.created_at,
        };
        record_request_log_detached(&self.state, event);
    }
}

/// 由本机请求上下文与配置推断客户端 IP：默认 peer；仅显式信任代理头时采信 XFF。
pub fn client_ip(
    peer: Option<std::net::SocketAddr>,
    headers: &axum::http::HeaderMap,
    settings: &ProxySettings,
) -> Option<String> {
    if settings.trust_proxy_headers
        && let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
        && let Some(first) = forwarded.split(',').next()
        && let Ok(ip) = first.trim().parse::<std::net::IpAddr>()
    {
        return Some(ip.to_string());
    }
    peer.map(|peer| peer.ip().to_string())
}

pub fn user_agent(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::USER_AGENT)?.to_str().ok()?;
    let cleaned: String = raw
        .chars()
        .filter(|ch| !ch.is_control())
        .take(512)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_constructors_carry_codes() {
        let outcome = Outcome {
            status: UsageStatus::Rejected,
            http_status: 403,
            error_code: Some("model_not_allowed".to_string()),
            input_tokens: None,
            output_tokens: None,
            usage_source: UsageSource::Missing,
            ttft_ms: None,
            response_size: None,
        };
        assert_eq!(outcome.status, UsageStatus::Rejected);
        assert_eq!(outcome.error_code.as_deref(), Some("model_not_allowed"));
    }

    #[test]
    fn user_agent_is_truncated_and_cleaned() {
        let mut headers = axum::http::HeaderMap::new();
        let long = format!("agent/{}", "x".repeat(600));
        headers.insert(
            axum::http::header::USER_AGENT,
            long.parse().expect("header value"),
        );
        let agent = user_agent(&headers).expect("agent");
        assert_eq!(agent.chars().count(), 512);
    }

    #[test]
    fn xff_only_used_when_trusted() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.7, 10.0.0.1".parse().expect("header"),
        );
        let mut settings = ProxySettings {
            max_body_bytes: 1024,
            max_upstream_json_bytes: 1024,
            headers_timeout: std::time::Duration::from_secs(1),
            non_stream_total_timeout: std::time::Duration::from_secs(1),
            stream_idle_timeout: std::time::Duration::from_secs(1),
            stream_max_duration: std::time::Duration::from_secs(1),
            max_attempts: 2,
            expose_provider_header: false,
            usd_cny_rate: rust_decimal::Decimal::ONE,
            trust_proxy_headers: false,
        };
        let peer: std::net::SocketAddr = "127.0.0.1:5000".parse().expect("addr");
        assert_eq!(
            client_ip(Some(peer), &headers, &settings).as_deref(),
            Some("127.0.0.1")
        );
        settings.trust_proxy_headers = true;
        assert_eq!(
            client_ip(Some(peer), &headers, &settings).as_deref(),
            Some("203.0.113.7")
        );
    }
}
