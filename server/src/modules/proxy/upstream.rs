//! 上游 HTTP 执行：请求构造、分层超时、错误体有界读取与清洗、错误分类（可重试/不可重试）。
//!
//! 设计：proxy 技术设计 §5.3–§5.5。S4 上游仅 `openai_compatible` `/chat/completions`；
//! 账号由 providers 选择，凭证只在内存中用于单次请求头。

use std::time::{Duration, Instant};

use bytes::Bytes;
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::{Response, StatusCode};

use crate::platform::error::{AppError, UpstreamProblem};
use crate::platform::state::AppState;

use super::error::{sanitize_upstream_code, upstream_trace_id};
use super::route::PlannedTarget;
use super::settings::{MAX_UPSTREAM_ERROR_BYTES, ProxySettings};

pub const UPSTREAM_USER_AGENT: &str = concat!("gw-server/", env!("CARGO_PKG_VERSION"));

pub struct UpstreamSpec {
    pub url: String,
    pub credential: String,
    pub body: Bytes,
    pub is_stream: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Timeout,
    Connect,
    Status,
    Protocol,
    BodyTooLarge,
}

#[derive(Debug)]
pub struct HttpFailure {
    pub retryable: bool,
    pub kind: FailureKind,
    pub status: Option<u16>,
    pub problem: Option<UpstreamProblem>,
    pub retry_after_secs: Option<u64>,
    pub detail: &'static str,
}

impl HttpFailure {
    fn new(kind: FailureKind, retryable: bool, detail: &'static str) -> Self {
        Self {
            retryable,
            kind,
            status: None,
            problem: None,
            retry_after_secs: None,
            detail,
        }
    }

    fn with_status(mut self, status: u16, problem: UpstreamProblem) -> Self {
        self.status = Some(status);
        self.problem = Some(problem);
        self
    }
}

pub struct JsonUpstream {
    pub body: Bytes,
}

pub struct StreamUpstream {
    pub response: Response,
    pub first_chunk: Option<Bytes>,
}

/// 构造上游请求并等待响应头；只有拿到 2xx 才返回响应（未读 body）。
async fn send(
    state: &AppState,
    settings: &ProxySettings,
    target: &PlannedTarget,
    spec: &UpstreamSpec,
) -> Result<Response, HttpFailure> {
    if target.provider_kind != "openai_compatible" {
        return Err(HttpFailure::new(
            FailureKind::Protocol,
            false,
            "S4 仅支持 openai_compatible 上游",
        ));
    }
    let mut request = state
        .http
        .post(&spec.url)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT_ENCODING, "identity")
        .header(USER_AGENT, UPSTREAM_USER_AGENT)
        .timeout(settings.non_stream_total_timeout);
    if spec.is_stream {
        request = request
            .header(ACCEPT, "text/event-stream")
            .timeout(settings.stream_max_duration);
    } else {
        request = request.header(ACCEPT, "application/json");
    }
    let authorization = format!("Bearer {}", spec.credential);
    let header = reqwest::header::HeaderValue::from_str(&authorization)
        .map_err(|_| HttpFailure::new(FailureKind::Protocol, false, "凭证包含非法 header 字符"))?;
    request = request
        .header(AUTHORIZATION, header)
        .body(spec.body.clone());

    let started = Instant::now();
    let response = tokio::time::timeout(settings.headers_timeout, request.send())
        .await
        .map_err(|_| HttpFailure::new(FailureKind::Timeout, true, "等待上游响应头超时"))?
        .map_err(classify_transport)?;
    let status = response.status();
    if status.is_success() {
        tracing::debug!(
            provider_id = %target.provider_id,
            upstream_model = %target.upstream_model,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "upstream accepted request"
        );
        return Ok(response);
    }
    Err(classify_status(response).await)
}

/// 把 reqwest 错误映射为分类失败。
fn classify_transport(error: reqwest::Error) -> HttpFailure {
    if error.is_timeout() {
        HttpFailure::new(FailureKind::Timeout, true, "上游请求超时")
    } else if error.is_connect() {
        HttpFailure::new(FailureKind::Connect, true, "连接上游失败")
    } else {
        HttpFailure::new(FailureKind::Connect, false, "上游连接/读取失败")
    }
}

async fn classify_status(response: Response) -> HttpFailure {
    let status = response.status();
    let headers = response.headers().clone();
    let trace_id = upstream_trace_id(&headers);
    let retry_after_secs = retry_after_secs(&headers);
    let body = read_bounded(response, MAX_UPSTREAM_ERROR_BYTES, Duration::from_secs(5))
        .await
        .unwrap_or_default();
    let (code, message) = parse_upstream_error(&body);
    // 上游 message 只写日志（截断、去控制字符）；绝不进响应。
    tracing::warn!(
        upstream_status = status.as_u16(),
        upstream_code = code.as_deref().unwrap_or("none"),
        upstream_trace_id = trace_id.as_deref().unwrap_or("none"),
        upstream_message = %log_safe(message.as_deref().unwrap_or("")),
        "upstream returned error"
    );
    let problem = UpstreamProblem {
        status: status.as_u16(),
        code,
        trace_id,
    };
    if status == StatusCode::TOO_MANY_REQUESTS {
        let mut failure = HttpFailure::new(FailureKind::Status, false, "上游限流")
            .with_status(status.as_u16(), problem);
        failure.retry_after_secs = Some(retry_after_secs.unwrap_or(60));
        return failure;
    }
    if status.is_server_error() {
        return HttpFailure::new(FailureKind::Status, true, "上游返回 5xx")
            .with_status(status.as_u16(), problem);
    }
    HttpFailure::new(FailureKind::Status, false, "上游返回 4xx")
        .with_status(status.as_u16(), problem)
}

/// 非流式：拿到 2xx 后按总超时读取有界 JSON。
pub async fn execute_json(
    state: &AppState,
    settings: &ProxySettings,
    target: &PlannedTarget,
    spec: &UpstreamSpec,
) -> Result<JsonUpstream, HttpFailure> {
    let response = send(state, settings, target, spec).await?;
    let headers = response.headers().clone();
    if !json_content_type(&headers) {
        return Err(HttpFailure::new(
            FailureKind::Protocol,
            false,
            "上游 content-type 不是 JSON",
        ));
    }
    let body = tokio::time::timeout(
        settings.non_stream_total_timeout,
        read_bounded(
            response,
            settings.max_upstream_json_bytes,
            settings.non_stream_total_timeout,
        ),
    )
    .await
    .map_err(|_| HttpFailure::new(FailureKind::Timeout, true, "读取上游响应超时"))?
    .map_err(|error| match error {
        BodyReadError::Timeout => HttpFailure::new(FailureKind::Timeout, true, "读取上游响应超时"),
        BodyReadError::TooLarge => {
            HttpFailure::new(FailureKind::BodyTooLarge, false, "上游响应超过网关上限")
        }
        BodyReadError::Transport => {
            HttpFailure::new(FailureKind::Protocol, false, "读取上游响应失败")
        }
    })?;
    Ok(JsonUpstream { body })
}

/// 流式：拿到 2xx 后校验 SSE content-type，并预读首个 chunk 以支持“流开始前失败可重试”。
pub async fn execute_stream(
    state: &AppState,
    settings: &ProxySettings,
    target: &PlannedTarget,
    spec: &UpstreamSpec,
) -> Result<StreamUpstream, HttpFailure> {
    let mut response = send(state, settings, target, spec).await?;
    let headers = response.headers().clone();
    if !is_event_stream(&headers) {
        return Err(HttpFailure::new(
            FailureKind::Protocol,
            false,
            "上游 content-type 不是 text/event-stream",
        ));
    }
    let first_chunk = tokio::time::timeout(settings.headers_timeout, response.chunk())
        .await
        .map_err(|_| HttpFailure::new(FailureKind::Timeout, true, "等待上游首字节超时"))?
        .map_err(|_| HttpFailure::new(FailureKind::Timeout, true, "读取上游首字节失败"))?;
    Ok(StreamUpstream {
        response,
        first_chunk,
    })
}

/// 可重试错误 → 客户端错误映射（已清洗，不泄露供应商/账号/凭证）。
pub fn map_failure(failure: HttpFailure) -> AppError {
    match failure.kind {
        FailureKind::Protocol => {
            AppError::bad_gateway("upstream_protocol_error", failure.detail, failure.problem)
        }
        FailureKind::Timeout => AppError::gateway_timeout(
            "upstream_timeout",
            "上游服务超时，请稍后重试",
            failure.problem,
        ),
        FailureKind::BodyTooLarge => AppError::bad_gateway(
            "upstream_protocol_error",
            "上游响应超过网关上限",
            failure.problem,
        ),
        FailureKind::Connect if failure.status.is_none() => AppError::bad_gateway(
            "upstream_error",
            "连接上游服务失败，请稍后重试",
            failure.problem,
        ),
        FailureKind::Status => match failure.status {
            Some(429) => AppError::throttled(
                "upstream_rate_limited",
                "上游服务限流，请稍后重试",
                failure.retry_after_secs.unwrap_or(60),
                None,
            ),
            Some(400 | 409 | 422) => AppError::bad_request(
                "upstream_invalid_request",
                "上游拒绝了请求参数，请检查请求内容",
            ),
            _ => AppError::bad_gateway(
                "upstream_error",
                "上游服务返回错误，请稍后重试并提供 x-request-id",
                failure.problem,
            ),
        },
        FailureKind::Connect => AppError::bad_gateway(
            "upstream_error",
            "上游服务连接异常，请稍后重试",
            failure.problem,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyReadError {
    Timeout,
    Transport,
    TooLarge,
}

async fn read_bounded(
    response: Response,
    limit: usize,
    timeout: Duration,
) -> Result<Bytes, BodyReadError> {
    let mut response = response;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut buf = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(BodyReadError::Timeout);
        }
        match tokio::time::timeout(remaining, response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                if buf.len() + chunk.len() > limit {
                    return Err(BodyReadError::TooLarge);
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(Ok(None)) => return Ok(Bytes::from(buf)),
            Ok(Err(error)) => {
                return Err(if error.is_timeout() {
                    BodyReadError::Timeout
                } else {
                    BodyReadError::Transport
                });
            }
            Err(_) => return Err(BodyReadError::Timeout),
        }
    }
}

fn json_content_type(headers: &reqwest::header::HeaderMap) -> bool {
    let Some(value) = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return true; // 兼容未返回 content-type 的兼容实现
    };
    let value = value.to_ascii_lowercase();
    value.contains("application/json") || value.contains("+json")
}

fn is_event_stream(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false)
}

fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let raw = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())?
        .trim();
    // 只支持秒数形式；HTTP-date 形式回退默认值。
    let secs = raw.parse::<u64>().ok()?;
    Some(secs.clamp(1, 300))
}

fn parse_upstream_error(body: &[u8]) -> (Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return (None, None);
    };
    let error = value.get("error").unwrap_or(&value);
    let code = error
        .get("code")
        .and_then(serde_json::Value::as_str)
        .or_else(|| error.get("type").and_then(serde_json::Value::as_str));
    let message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    (sanitize_upstream_code(code), message)
}

fn log_safe(raw: &str) -> String {
    raw.chars()
        .filter(|ch| !ch.is_control())
        .take(512)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_upstream_error_codes_and_messages() {
        let body = br#"{"error":{"code":"server_error","message":"boom"}}"#;
        let (code, message) = parse_upstream_error(body);
        assert_eq!(code.as_deref(), Some("server_error"));
        assert_eq!(message.as_deref(), Some("boom"));
        let (code, message) = parse_upstream_error(br#"{"type":"overloaded_error"}"#);
        assert_eq!(code.as_deref(), Some("overloaded_error"));
        assert!(message.is_none());
        assert_eq!(parse_upstream_error(b"not-json"), (None, None));
    }

    #[test]
    fn retry_after_is_clamped() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "999".parse().expect("header"));
        assert_eq!(retry_after_secs(&headers), Some(300));
        headers.insert(reqwest::header::RETRY_AFTER, "0".parse().expect("header"));
        assert_eq!(retry_after_secs(&headers), Some(1));
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "soon".parse().expect("header"),
        );
        assert_eq!(retry_after_secs(&headers), None);
    }

    #[test]
    fn content_type_checks() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert!(json_content_type(&headers)); // 缺失时兼容
        headers.insert(
            CONTENT_TYPE,
            "application/json; charset=utf-8".parse().expect("header"),
        );
        assert!(json_content_type(&headers));
        assert!(!is_event_stream(&headers));
        headers.insert(CONTENT_TYPE, "text/event-stream".parse().expect("header"));
        assert!(is_event_stream(&headers));
        assert!(!json_content_type(&headers));
    }
}
