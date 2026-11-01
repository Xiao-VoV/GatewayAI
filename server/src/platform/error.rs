//! 统一错误 → RFC 7807 problem+json。
//!
//! 设计见 `设计文档（暂未开源）` §1.3/§7 与
//! `设计文档（暂未开源）` §1.3：
//! - 字段：`type/title/status/code/detail/errors?/references?`
//! - 所有错误响应带 `Cache-Control: no-store`
//! - 429 带 `Retry-After`
//! - 500 固定文案，不返回内部细节（明细只写日志）

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::BTreeMap;
use thiserror::Error;

/// 上游错误清洗后的暴露字段（不含上游 message/凭证/账号）。
#[derive(Debug, Clone)]
pub struct UpstreamProblem {
    pub status: u16,
    pub code: Option<String>,
    pub trace_id: Option<String>,
}

/// 预算超限的可读扩展字段（CNY，6 位小数字符串序列化）。
#[derive(Debug, Clone)]
pub struct BudgetProblem {
    pub daily_limit: Decimal,
    pub spent: Decimal,
    pub estimated: Decimal,
    pub currency: String,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not implemented: {0}")]
    #[allow(dead_code)] // 501 占位体系保留（后续未实现端点/兼容）
    NotImplemented(&'static str),
    #[error("validation failed")]
    Validation { errors: BTreeMap<String, String> },
    #[error("{code}")]
    Unauthorized {
        code: &'static str,
        detail: &'static str,
    },
    #[error("{code}")]
    Forbidden { code: &'static str, detail: String },
    #[error("{code}")]
    Conflict {
        code: &'static str,
        detail: String,
        references: Option<BTreeMap<String, u64>>,
    },
    #[error("{code}")]
    BadRequest { code: &'static str, detail: String },
    #[error("{code}")]
    NotFound { code: &'static str, detail: String },
    #[error("too many attempts")]
    TooManyAttempts { retry_after_secs: u64 },
    #[error("database unavailable")]
    DatabaseUnavailable,
    #[error("credential unavailable")]
    CredentialUnavailable,
    #[error("payload too large ({max_bytes} bytes)")]
    PayloadTooLarge { max_bytes: usize },
    #[error("{code}: {detail}")]
    UnsupportedMediaType { code: &'static str, detail: String },
    #[error("{code}: {detail}")]
    Throttled {
        code: &'static str,
        detail: String,
        retry_after_secs: u64,
        budget: Option<BudgetProblem>,
    },
    #[error("{code}: {detail}")]
    BadGateway {
        code: &'static str,
        detail: String,
        upstream: Option<UpstreamProblem>,
    },
    #[error("{code}: {detail}")]
    GatewayTimeout {
        code: &'static str,
        detail: String,
        upstream: Option<UpstreamProblem>,
    },
    #[error("model not found")]
    ModelNotFound {
        detail: String,
        available_models: Vec<String>,
    },
    #[error("internal error")]
    Internal,
}

impl AppError {
    #[allow(dead_code)] // 501 占位体系保留（后续未实现端点/兼容）
    pub fn not_implemented(detail: &'static str) -> Self {
        Self::NotImplemented(detail)
    }

    pub fn validation(errors: BTreeMap<String, String>) -> Self {
        Self::Validation { errors }
    }

    pub fn validation_field(field: &str, message: impl Into<String>) -> Self {
        let mut errors = BTreeMap::new();
        errors.insert(field.to_string(), message.into());
        Self::Validation { errors }
    }

    pub fn invalid_credentials() -> Self {
        Self::Unauthorized {
            code: "invalid_credentials",
            detail: "邮箱或密码错误",
        }
    }

    pub fn not_authenticated() -> Self {
        Self::Unauthorized {
            code: "not_authenticated",
            detail: "请先登录",
        }
    }

    pub fn csrf_invalid() -> Self {
        Self::Forbidden {
            code: "csrf_invalid",
            detail: "CSRF token 不正确或已过期".to_string(),
        }
    }

    pub fn account_disabled() -> Self {
        Self::Forbidden {
            code: "account_disabled",
            detail: "该管理员已被禁用".to_string(),
        }
    }

    pub fn setup_already_completed() -> Self {
        Self::Forbidden {
            code: "setup_already_completed",
            detail: "系统已有管理员，不能再初始化".to_string(),
        }
    }

    pub fn setup_token_invalid() -> Self {
        Self::Forbidden {
            code: "setup_token_invalid",
            detail: "setup token 缺失或不正确".to_string(),
        }
    }

    pub fn setup_conflict() -> Self {
        Self::Conflict {
            code: "setup_conflict",
            detail: "已存在管理员，请直接登录".to_string(),
            references: None,
        }
    }

    pub fn bad_request(code: &'static str, detail: impl Into<String>) -> Self {
        Self::BadRequest {
            code,
            detail: detail.into(),
        }
    }

    pub fn not_found(code: &'static str, detail: impl Into<String>) -> Self {
        Self::NotFound {
            code,
            detail: detail.into(),
        }
    }

    pub fn conflict(
        code: &'static str,
        detail: impl Into<String>,
        references: Option<BTreeMap<String, u64>>,
    ) -> Self {
        Self::Conflict {
            code,
            detail: detail.into(),
            references,
        }
    }

    pub fn too_many_attempts(retry_after_secs: u64) -> Self {
        Self::TooManyAttempts { retry_after_secs }
    }

    pub fn database_unavailable() -> Self {
        Self::DatabaseUnavailable
    }

    pub fn credential_unavailable() -> Self {
        Self::CredentialUnavailable
    }

    pub fn payload_too_large(max_bytes: usize) -> Self {
        Self::PayloadTooLarge { max_bytes }
    }

    pub fn unsupported_media_type(code: &'static str, detail: impl Into<String>) -> Self {
        Self::UnsupportedMediaType {
            code,
            detail: detail.into(),
        }
    }

    pub fn throttled(
        code: &'static str,
        detail: impl Into<String>,
        retry_after_secs: u64,
        budget: Option<BudgetProblem>,
    ) -> Self {
        Self::Throttled {
            code,
            detail: detail.into(),
            retry_after_secs,
            budget,
        }
    }

    pub fn bad_gateway(
        code: &'static str,
        detail: impl Into<String>,
        upstream: Option<UpstreamProblem>,
    ) -> Self {
        Self::BadGateway {
            code,
            detail: detail.into(),
            upstream,
        }
    }

    pub fn gateway_timeout(
        code: &'static str,
        detail: impl Into<String>,
        upstream: Option<UpstreamProblem>,
    ) -> Self {
        Self::GatewayTimeout {
            code,
            detail: detail.into(),
            upstream,
        }
    }

    pub fn model_not_found(detail: impl Into<String>, available_models: Vec<String>) -> Self {
        Self::ModelNotFound {
            detail: detail.into(),
            available_models,
        }
    }

    /// 未预期错误：明细只进日志，响应固定文案。
    pub fn internal<E: std::fmt::Display>(error: E) -> Self {
        tracing::error!(error = %error, "unexpected internal error");
        Self::Internal
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
                tracing::warn!(error = %error, "database unavailable");
                Self::DatabaseUnavailable
            }
            _ => Self::internal(error),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status;
        let code;
        let detail;
        let mut errors = None;
        let mut references = None;
        let mut retry_after = None;
        let mut budget = None;
        let mut upstream = None;
        let mut available_models = None;

        match self {
            Self::NotImplemented(detail_str) => {
                status = StatusCode::NOT_IMPLEMENTED;
                code = "not_implemented";
                detail = detail_str.to_string();
            }
            Self::Validation {
                errors: field_errors,
            } => {
                status = StatusCode::BAD_REQUEST;
                code = "validation_failed";
                detail = "请求参数不合法".to_string();
                errors = Some(field_errors);
            }
            Self::Unauthorized { code: c, detail: d } => {
                status = StatusCode::UNAUTHORIZED;
                code = c;
                detail = d.to_string();
            }
            Self::Forbidden { code: c, detail: d } => {
                status = StatusCode::FORBIDDEN;
                code = c;
                detail = d;
            }
            Self::Conflict {
                code: c,
                detail: d,
                references: refs,
            } => {
                status = StatusCode::CONFLICT;
                code = c;
                detail = d;
                references = refs;
            }
            Self::BadRequest { code: c, detail: d } => {
                status = StatusCode::BAD_REQUEST;
                code = c;
                detail = d;
            }
            Self::NotFound { code: c, detail: d } => {
                status = StatusCode::NOT_FOUND;
                code = c;
                detail = d;
            }
            Self::TooManyAttempts { retry_after_secs } => {
                status = StatusCode::TOO_MANY_REQUESTS;
                code = "too_many_attempts";
                detail = "尝试过于频繁，请稍后再试".to_string();
                retry_after = Some(retry_after_secs);
            }
            Self::PayloadTooLarge { max_bytes } => {
                status = StatusCode::PAYLOAD_TOO_LARGE;
                code = "request_too_large";
                detail = format!("请求体超过上限（{max_bytes} bytes）");
            }
            Self::UnsupportedMediaType { code: c, detail: d } => {
                status = StatusCode::UNSUPPORTED_MEDIA_TYPE;
                code = c;
                detail = d;
            }
            Self::Throttled {
                code: c,
                detail: d,
                retry_after_secs: secs,
                budget: budget_problem,
            } => {
                status = StatusCode::TOO_MANY_REQUESTS;
                code = c;
                detail = d;
                retry_after = Some(secs);
                budget = budget_problem;
            }
            Self::BadGateway {
                code: c,
                detail: d,
                upstream: problem,
            } => {
                status = StatusCode::BAD_GATEWAY;
                code = c;
                detail = d;
                upstream = problem;
            }
            Self::GatewayTimeout {
                code: c,
                detail: d,
                upstream: problem,
            } => {
                status = StatusCode::GATEWAY_TIMEOUT;
                code = c;
                detail = d;
                upstream = problem;
            }
            Self::ModelNotFound {
                detail: d,
                available_models: models,
            } => {
                status = StatusCode::NOT_FOUND;
                code = "model_not_found";
                detail = d;
                available_models = if models.is_empty() {
                    None
                } else {
                    Some(models)
                };
            }
            Self::DatabaseUnavailable => {
                status = StatusCode::SERVICE_UNAVAILABLE;
                code = "database_unavailable";
                detail = "数据库暂不可用".to_string();
            }
            Self::CredentialUnavailable => {
                status = StatusCode::INTERNAL_SERVER_ERROR;
                code = "credential_unavailable";
                detail = "凭证暂不可用，请联系运维检查主密钥".to_string();
            }
            Self::Internal => {
                status = StatusCode::INTERNAL_SERVER_ERROR;
                code = "internal_error";
                detail = "服务器内部错误，请稍后重试并提供 x-request-id".to_string();
            }
        }

        let mut body = json!({
            "type": "about:blank",
            "title": status.canonical_reason().unwrap_or("Error"),
            "status": status.as_u16(),
            "code": code,
            "detail": detail,
        });
        if let Some(errors) = errors
            && !errors.is_empty()
        {
            body["errors"] = serde_json::to_value(errors).unwrap_or_default();
        }
        if let Some(references) = references
            && !references.is_empty()
        {
            body["references"] = serde_json::to_value(references).unwrap_or_default();
        }
        if let Some(budget) = budget {
            body["budget"] = json!({
                "daily_limit": decimal_6(budget.daily_limit),
                "spent": decimal_6(budget.spent),
                "estimated": decimal_6(budget.estimated),
                "currency": budget.currency,
            });
        }
        if let Some(upstream) = upstream {
            let mut value = json!({"status": upstream.status});
            if let Some(code) = upstream.code {
                value["code"] = json!(code);
            }
            if let Some(trace_id) = upstream.trace_id {
                value["trace_id"] = json!(trace_id);
            }
            body["upstream"] = value;
        }
        if let Some(models) = available_models {
            body["available_models"] = json!(models);
        }

        let mut response = (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            Json(body),
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if let Some(secs) = retry_after
            && let Ok(value) = HeaderValue::from_str(&secs.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

fn decimal_6(value: Decimal) -> String {
    format!("{value:.6}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use rust_decimal::Decimal;

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    #[tokio::test]
    async fn budget_throttled_has_problem_extensions_and_retry_after() {
        let response = AppError::throttled(
            "budget_exceeded",
            "今日预算已用尽",
            60,
            Some(BudgetProblem {
                daily_limit: Decimal::new(500_000_000, 8),
                spent: Decimal::new(499_000_000, 8),
                estimated: Decimal::new(2_000_000, 8),
                currency: "CNY".to_string(),
            }),
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("60")
        );
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        let body = body_json(response).await;
        assert_eq!(body["code"], "budget_exceeded");
        assert_eq!(body["budget"]["daily_limit"], "5.000000");
        assert_eq!(body["budget"]["spent"], "4.990000");
        assert_eq!(body["budget"]["estimated"], "0.020000");
        assert_eq!(body["budget"]["currency"], "CNY");
    }

    #[tokio::test]
    async fn upstream_and_model_not_found_extensions() {
        let response = AppError::bad_gateway(
            "upstream_error",
            "上游服务返回错误",
            Some(UpstreamProblem {
                status: 500,
                code: Some("server_error".to_string()),
                trace_id: Some("req_upstream".to_string()),
            }),
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = body_json(response).await;
        assert_eq!(body["upstream"]["status"], 500);
        assert_eq!(body["upstream"]["code"], "server_error");
        assert_eq!(body["upstream"]["trace_id"], "req_upstream");

        let response =
            AppError::model_not_found("模型不存在", vec!["fast".to_string(), "smart".to_string()])
                .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            body_json(response).await["available_models"],
            serde_json::json!(["fast", "smart"])
        );
    }

    #[tokio::test]
    async fn payload_and_media_type_errors_are_problem_json() {
        let response = AppError::payload_too_large(8 * 1024 * 1024).into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body_json(response).await["code"], "request_too_large");

        let response = AppError::unsupported_media_type(
            "unsupported_media_type",
            "Content-Type 必须是 application/json",
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(body_json(response).await["code"], "unsupported_media_type");
    }
}
