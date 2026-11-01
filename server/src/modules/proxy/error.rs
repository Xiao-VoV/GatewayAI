//! proxy 错误构造：problem+json 映射、上游错误清洗、SSE 错误帧。
//!
//! 平台 `AppError` 提供 413/415/429/502/504/404-model_not_found 等变体；proxy 另外需要
//! 少量平台没有的 503（如 `model_unavailable`），用 [`ProxyError::Custom`] 构造同形状 problem+json。
//! 本模块不复制平台序列化逻辑（App 变体直接委托 `AppError::into_response`）。

use axum::{
    Json,
    http::{HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::platform::error::AppError;

/// 数据面错误：平台 AppError 或 proxy 自定义 problem（仍是同一 RFC 7807 形状）。
#[derive(Debug)]
pub enum ProxyError {
    App(AppError),
    Custom {
        status: StatusCode,
        code: &'static str,
        detail: String,
    },
}

impl From<AppError> for ProxyError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl ProxyError {
    pub fn custom(status: StatusCode, code: &'static str, detail: impl Into<String>) -> Self {
        Self::Custom {
            status,
            code,
            detail: detail.into(),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::App(error) => app_status(error),
            Self::Custom { status, .. } => *status,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::App(error) => app_code(error),
            Self::Custom { code, .. } => code,
        }
    }

    pub fn into_response(self) -> Response {
        match self {
            Self::App(error) => error.into_response(),
            Self::Custom {
                status,
                code,
                detail,
            } => {
                let body = json!({
                    "type": "about:blank",
                    "title": status.canonical_reason().unwrap_or("Error"),
                    "status": status.as_u16(),
                    "code": code,
                    "detail": detail,
                });
                let mut response = (
                    status,
                    [(header::CONTENT_TYPE, "application/problem+json")],
                    Json(body),
                )
                    .into_response();
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            }
        }
    }
}

fn app_status(error: &AppError) -> StatusCode {
    match error {
        AppError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
        AppError::Validation { .. } | AppError::BadRequest { .. } => StatusCode::BAD_REQUEST,
        AppError::Unauthorized { .. } => StatusCode::UNAUTHORIZED,
        AppError::Forbidden { .. } => StatusCode::FORBIDDEN,
        AppError::Conflict { .. } => StatusCode::CONFLICT,
        AppError::NotFound { .. } | AppError::ModelNotFound { .. } => StatusCode::NOT_FOUND,
        AppError::TooManyAttempts { .. } | AppError::Throttled { .. } => {
            StatusCode::TOO_MANY_REQUESTS
        }
        AppError::DatabaseUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        AppError::CredentialUnavailable | AppError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        AppError::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        AppError::UnsupportedMediaType { .. } => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        AppError::BadGateway { .. } => StatusCode::BAD_GATEWAY,
        AppError::GatewayTimeout { .. } => StatusCode::GATEWAY_TIMEOUT,
    }
}

fn app_code(error: &AppError) -> &'static str {
    match error {
        AppError::NotImplemented(_) => "not_implemented",
        AppError::Validation { .. } => "validation_failed",
        AppError::Unauthorized { code, .. } => code,
        AppError::Forbidden { code, .. } => code,
        AppError::Conflict { code, .. } => code,
        AppError::BadRequest { code, .. } => code,
        AppError::NotFound { code, .. } => code,
        AppError::ModelNotFound { .. } => "model_not_found",
        AppError::TooManyAttempts { .. } => "too_many_attempts",
        AppError::Throttled { code, .. } => code,
        AppError::DatabaseUnavailable => "database_unavailable",
        AppError::CredentialUnavailable => "credential_unavailable",
        AppError::Internal => "internal_error",
        AppError::PayloadTooLarge { .. } => "request_too_large",
        AppError::UnsupportedMediaType { code, .. } => code,
        AppError::BadGateway { code, .. } => code,
        AppError::GatewayTimeout { code, .. } => code,
    }
}

/// 上游协议/内容类型不符（不重试）。
pub fn upstream_protocol(detail: impl Into<String>) -> ProxyError {
    ProxyError::from(AppError::bad_gateway(
        "upstream_protocol_error",
        detail,
        None,
    ))
}

/// 无可用上游候选/账号（503，平台 AppError 无该 code，用自定义 problem）。
pub fn model_unavailable() -> ProxyError {
    ProxyError::custom(
        StatusCode::SERVICE_UNAVAILABLE,
        "model_unavailable",
        "无可用上游模型/账号，请稍后重试或联系管理员",
    )
}

/// 把上游响应头里的 trace id 提取为安全字符串（≤128，白名单字符）。
pub fn upstream_trace_id(headers: &reqwest::header::HeaderMap) -> Option<String> {
    for name in [
        "x-request-id",
        "request-id",
        "openai-request-id",
        "anthropic-request-id",
    ] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            let value = value.trim();
            if (1..=128).contains(&value.len())
                && value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
            {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 上游 JSON 错误体的 `error.code`/`error.type` 清洗（≤64，白名单字符）。
pub fn sanitize_upstream_code(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    let cleaned: String = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
        .take(64)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// 数据面错误响应：problem+json，已解析路由时补 x-gw-* 响应头，统一 no-store；
/// 401 补 `WWW-Authenticate`。
pub struct DataPlaneError {
    pub error: ProxyError,
    pub request_id: Option<String>,
    pub resolved_model: Option<String>,
    pub fallback: bool,
    pub provider: Option<String>,
    pub model_group: Option<String>,
    pub catalog_version: Option<String>,
}

impl DataPlaneError {
    pub fn new(error: impl Into<ProxyError>) -> Self {
        Self {
            error: error.into(),
            request_id: None,
            resolved_model: None,
            fallback: false,
            provider: None,
            model_group: None,
            catalog_version: None,
        }
    }

    pub fn with_route(
        mut self,
        request_id: Option<String>,
        resolved_model: Option<String>,
        fallback: bool,
        provider: Option<String>,
    ) -> Self {
        self.request_id = request_id;
        self.resolved_model = resolved_model;
        self.fallback = fallback;
        self.provider = provider;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_catalog(
        mut self,
        model_group: Option<String>,
        catalog_version: Option<String>,
    ) -> Self {
        self.model_group = model_group;
        self.catalog_version = catalog_version;
        self
    }

    pub fn into_response(self) -> Response {
        let status = self.error.status();
        let mut response = self.error.into_response();
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if let Some(request_id) = self.request_id
            && let Ok(value) = HeaderValue::from_str(&request_id)
        {
            headers.insert(HeaderName::from_static("x-request-id"), value);
        }
        if let Some(model) = self.resolved_model
            && let Ok(value) = HeaderValue::from_str(&model)
        {
            headers.insert(HeaderName::from_static("x-gw-resolved-model"), value);
        }
        if self.fallback {
            headers.insert(
                HeaderName::from_static("x-gw-fallback"),
                HeaderValue::from_static("true"),
            );
        }
        if let Some(provider) = self.provider
            && let Ok(value) = HeaderValue::from_str(&provider)
        {
            headers.insert(HeaderName::from_static("x-gw-provider"), value);
        }
        if let Some(model_group) = self.model_group
            && let Ok(value) = HeaderValue::from_str(&model_group)
        {
            headers.insert(HeaderName::from_static("x-gw-model-group"), value);
        }
        if let Some(catalog_version) = self.catalog_version
            && let Ok(value) = HeaderValue::from_str(&catalog_version)
        {
            headers.insert(HeaderName::from_static("x-gw-catalog-version"), value);
        }
        if status == StatusCode::UNAUTHORIZED {
            headers.insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"gw\""),
            );
        }
        response
    }
}

/// SSE 错误帧（Chat 前端形状）。
pub fn chat_stream_error_frame(message: &str, kind: &str, code: &str) -> bytes::Bytes {
    let value = json!({
        "error": {
            "message": message,
            "type": kind,
            "code": code,
            "param": null
        }
    });
    super::sse::json_frame(None, &value)
}

/// SSE 错误帧（OpenAI Responses 前端形状；不带 [DONE]，以 error 结束）。
pub fn responses_stream_error_frame(message: &str) -> bytes::Bytes {
    let value = json!({
        "type": "error",
        "error": { "type": "api_error", "message": message }
    });
    super::sse::json_frame(Some("error"), &value)
}

/// Gemini SSE 流内错误：Google 风格 error 对象（`data: {...}`，无 event 名）。
pub fn gemini_stream_error_frame(message: &str) -> bytes::Bytes {
    let value = json!({
        "error": { "code": 502, "message": message, "status": "UPSTREAM_ERROR" }
    });
    super::sse::json_frame(None, &value)
}

/// SSE 错误帧（Anthropic Messages 前端形状）。
pub fn messages_stream_error_frame(kind: &str, message: &str) -> bytes::Bytes {
    let value = json!({
        "type": "error",
        "error": { "type": kind, "message": message }
    });
    super::sse::json_frame(Some("error"), &value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_upstream_codes() {
        assert_eq!(
            sanitize_upstream_code(Some("server_error")).as_deref(),
            Some("server_error")
        );
        assert_eq!(
            sanitize_upstream_code(Some("bad code<script>")).as_deref(),
            Some("badcodescript")
        );
        assert_eq!(sanitize_upstream_code(Some("   ")), None);
    }

    #[test]
    fn sanitizes_trace_id() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-request-id", "req_abc-123".parse().expect("header"));
        assert_eq!(upstream_trace_id(&headers).as_deref(), Some("req_abc-123"));
        headers.insert("x-request-id", "bad value".parse().expect("header"));
        assert_eq!(upstream_trace_id(&headers), None);
    }

    #[test]
    fn custom_problem_has_expected_shape() {
        let response = model_unavailable().into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
    }

    #[test]
    fn stream_error_frames_have_protocol_shapes() {
        let chat = String::from_utf8(
            chat_stream_error_frame("失败", "upstream_error", "upstream_error").to_vec(),
        )
        .expect("utf8");
        assert!(chat.contains("\"error\""));
        assert!(chat.contains("\"code\":\"upstream_error\""));
        let messages =
            String::from_utf8(messages_stream_error_frame("overloaded_error", "失败").to_vec())
                .expect("utf8");
        assert!(messages.starts_with("event: error\n"));
        assert!(messages.contains("\"type\":\"error\""));
    }
}
