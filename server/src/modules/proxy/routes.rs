//! 数据面路由 handler：`POST /v1/chat/completions`、`POST /v1/messages`、`POST /v1/responses`。
//!
//! 执行顺序（proxy 技术设计 §3.2）：Content-Type/Encoding → 读 body/上限 → 鉴权 →
//! JSON/字段校验 → Key 白名单 → routing 解析 → 预算 → 上游尝试（≤2）→ 响应/流 → 终结记账。
//! 错误统一 problem+json + `x-gw-*` 头；SSE 开始后的错误用前端原生错误事件。

use std::net::SocketAddr;

use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, header},
    response::{IntoResponse, Response},
    routing::post,
};
use futures_util::StreamExt;
use serde::Deserialize;
use uuid::Uuid;

use crate::modules::{
    catalog::{
        model::{CatalogError, CatalogSubject, ModelAuthorization},
        service as catalog_service,
    },
    metering::cost::estimate_tokens,
    metering::model::{UsageSource, UsageStatus},
    policy::rate_limit,
    routing::{model::RouteError, service as routing_service},
};
use crate::platform::{error::AppError, http::request_id_from_headers, state::AppState};

use super::{
    adapters::{
        FrontendProtocol,
        chat::{self, ChatRequest},
        gemini::{self, GeminiRequest},
        messages::{self, MessagesRequest},
        responses::{self, ResponsesRequest},
    },
    auth,
    budget::{self},
    error::{DataPlaneError, ProxyError, model_unavailable},
    executor::{self, ExecutionFailure, PreparedBody},
    route::{PlannedRoute, PlannedTarget},
    settings::ProxySettings,
    stream,
    telemetry::{Outcome, Telemetry, client_ip, user_agent},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(messages))
        .route("/v1/responses", post(responses))
        .route("/v1beta/models/{model_action}", post(gemini_generate))
}

async fn chat_completions(State(state): State<AppState>, request: Request) -> Response {
    handle(state, request, FrontendProtocol::ChatCompletions).await
}

async fn messages(State(state): State<AppState>, request: Request) -> Response {
    handle(state, request, FrontendProtocol::Messages).await
}

async fn responses(State(state): State<AppState>, request: Request) -> Response {
    handle(state, request, FrontendProtocol::Responses).await
}

#[derive(Debug, Deserialize)]
struct GeminiQuery {
    #[serde(default)]
    alt: Option<String>,
}

async fn gemini_generate(
    State(state): State<AppState>,
    Path(model_action): Path<String>,
    Query(query): Query<GeminiQuery>,
    request: Request,
) -> Response {
    let Some((model, action)) = model_action.rsplit_once(':') else {
        return gemini_early_error(
            &request,
            AppError::not_found("model_not_found", "模型或动作不存在"),
        );
    };
    let is_stream = match action {
        "generateContent" => false,
        "streamGenerateContent" => {
            let alt_ok = query
                .alt
                .as_deref()
                .is_some_and(|alt| alt.eq_ignore_ascii_case("sse"));
            if !alt_ok {
                return gemini_early_error(
                    &request,
                    AppError::bad_request(
                        "unsupported_parameter",
                        "streamGenerateContent 必须使用 alt=sse",
                    ),
                );
            }
            true
        }
        _ => {
            return gemini_early_error(
                &request,
                AppError::not_found("model_not_found", "模型或动作不存在"),
            );
        }
    };
    handle_meta(
        state,
        request,
        FrontendProtocol::Gemini,
        GEMINI_PATH,
        Some(model.to_string()),
        Some(is_stream),
    )
    .await
}

fn gemini_early_error(request: &Request, error: AppError) -> Response {
    let headers = request.headers().clone();
    let request_id = request_id_from_headers(&headers);
    DataPlaneError::new(ProxyError::from(error))
        .with_route(request_id, None, false, None)
        .into_response()
}

const CHAT_PATH: &str = "/v1/chat/completions";
const MESSAGES_PATH: &str = "/v1/messages";
const RESPONSES_PATH: &str = "/v1/responses";
/// 观测日志使用路由模板（真实模型名/suffix 不落库；observability 会归一）。
const GEMINI_PATH: &str = "/v1beta/models";

async fn handle(state: AppState, request: Request, frontend: FrontendProtocol) -> Response {
    let path = match frontend {
        FrontendProtocol::ChatCompletions => CHAT_PATH,
        FrontendProtocol::Messages => MESSAGES_PATH,
        FrontendProtocol::Responses => RESPONSES_PATH,
        FrontendProtocol::Gemini => GEMINI_PATH,
    };
    handle_meta(state, request, frontend, path, None, None).await
}

async fn handle_meta(
    state: AppState,
    request: Request,
    frontend: FrontendProtocol,
    path: &'static str,
    model_override: Option<String>,
    stream_override: Option<bool>,
) -> Response {
    let settings = ProxySettings::from_config(&state.config);
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0);
    let headers = request.headers().clone();
    let request_id =
        request_id_from_headers(&headers).unwrap_or_else(|| Uuid::new_v4().to_string());
    let ip = client_ip(peer, &headers, &settings);
    let ua = user_agent(&headers);
    let telemetry = Telemetry::new(
        state.clone(),
        request_id,
        frontend,
        "POST",
        path,
        ip,
        ua,
        false,
    );
    match handle_inner(
        state,
        settings.clone(),
        headers,
        request,
        telemetry,
        frontend,
        model_override,
        stream_override,
    )
    .await
    {
        Ok(response) => response,
        Err(failure) => failure.render(settings.expose_provider_header),
    }
}

#[allow(clippy::result_large_err, clippy::too_many_arguments)]
async fn handle_inner(
    state: AppState,
    settings: ProxySettings,
    headers: HeaderMap,
    request: Request,
    mut telemetry: Telemetry,
    frontend: FrontendProtocol,
    model_override: Option<String>,
    stream_override: Option<bool>,
) -> Result<Response, Failure> {
    // 1. Content-Type / Content-Encoding（鉴权前；无身份时只写请求日志）。
    if !content_type_ok(&headers) {
        return Err(Failure::from_error(
            telemetry,
            ProxyError::from(AppError::unsupported_media_type(
                "unsupported_media_type",
                "Content-Type 必须是 application/json",
            )),
        ));
    }
    if let Some(encoding) = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        && !encoding.trim().is_empty()
        && !encoding.trim().eq_ignore_ascii_case("identity")
    {
        return Err(Failure::from_error(
            telemetry,
            ProxyError::from(AppError::unsupported_media_type(
                "unsupported_content_encoding",
                "请求体不支持压缩编码（Content-Encoding 必须为 identity）",
            )),
        ));
    }

    // 2. 读取 body（上限 413）。
    let body = match read_body(request, settings.max_body_bytes).await {
        Ok(body) => body,
        Err(BodyReadError::TooLarge) => {
            return Err(Failure::from_error(
                telemetry,
                ProxyError::from(AppError::payload_too_large(settings.max_body_bytes)),
            ));
        }
        Err(BodyReadError::Read) => {
            return Err(Failure::from_error(
                telemetry,
                ProxyError::from(AppError::validation_field("body", "读取请求体失败")),
            ));
        }
    };

    // 3. 数据面双因子鉴权（access 共享入口）。
    let auth_context = match auth::authenticate(&state, &headers).await {
        Ok(context) => context,
        Err(error) => return Err(Failure::from_error(telemetry, ProxyError::from(error))),
    };
    telemetry.with_auth(auth_context.clone());

    // 4. 前端协议解析/校验（适配器）。
    let parsed = match frontend {
        FrontendProtocol::ChatCompletions => chat::parse_request(&body)
            .map(ParsedRequest::Chat)
            .map_err(ProxyError::from),
        FrontendProtocol::Messages => messages::parse_request(&body)
            .map(ParsedRequest::Messages)
            .map_err(ProxyError::from),
        FrontendProtocol::Responses => responses::parse_request(&body)
            .map(ParsedRequest::Responses)
            .map_err(ProxyError::from),
        FrontendProtocol::Gemini => gemini::parse_request(
            &body,
            model_override.as_deref().unwrap_or_default(),
            stream_override.unwrap_or(false),
        )
        .map(ParsedRequest::Gemini)
        .map_err(ProxyError::from),
    };
    let parsed = match parsed {
        Ok(parsed) => parsed,
        Err(error) => return Err(Failure::from_error(telemetry, error)),
    };
    telemetry.is_stream = parsed.is_stream();
    telemetry.with_model_alias(parsed.alias().to_string());
    telemetry.with_adapter(parsed.adapter());

    // 5. Key 白名单。
    if !auth::model_allowed(&auth_context, parsed.alias()) {
        return Err(Failure::from_error(
            telemetry,
            ProxyError::from(AppError::Forbidden {
                code: "model_not_allowed",
                detail: "当前 Key 不允许调用该模型".to_string(),
            }),
        ));
    }

    // 5.1 Key RPM 限流（access 鉴权后、catalog 授权/routing 前；US-011）。
    // 429 仍写 request_logs（便于审计/观测），但不写 usage、不占上游调用。
    let rate = rate_limit::check_key_rpm(auth_context.key_id, auth_context.key_rpm_limit);
    if !rate.allowed {
        telemetry.write_usage = false;
        return Err(Failure::from_error(
            telemetry,
            ProxyError::from(AppError::throttled(
                "key_rate_limited",
                "请求过于频繁，请稍后重试",
                rate.retry_after_secs,
                None,
            )),
        ));
    }

    // 5.2 catalog 发布授权（在 routing 前；catalog active 组同名未发布不得 fallback alias）。
    let subject = CatalogSubject {
        user_id: auth_context.user_id,
        device_id: Some(auth_context.device_id),
        user_group_ids: auth_context.user_group_ids.clone(),
        key_allowed_models: auth_context.allowed_models.clone(),
    };
    match catalog_service::authorize_model(&state, &subject, parsed.alias()).await {
        Ok(ModelAuthorization::Allowed | ModelAuthorization::Unknown) => {}
        Ok(ModelAuthorization::VisibleOnly) => {
            return Err(Failure::from_error(
                telemetry,
                ProxyError::from(AppError::Forbidden {
                    code: "model_not_allowed",
                    detail: "该模型目录可见，但当前用户/Key 未获授权调用".to_string(),
                }),
            ));
        }
        Ok(ModelAuthorization::NotPublished) => {
            return Err(Failure::from_error(
                telemetry,
                ProxyError::from(AppError::Forbidden {
                    code: "model_not_allowed",
                    detail: "该模型未发布给当前用户/设备".to_string(),
                }),
            ));
        }
        Err(CatalogError::DatabaseUnavailable) => {
            return Err(Failure::from_error(
                telemetry,
                ProxyError::from(AppError::database_unavailable()),
            ));
        }
    }

    // 6. routing 解析（catalog-first → legacy alias 回退）。
    let plan = match routing_service::resolve_model(&state, parsed.alias()).await {
        Ok(plan) => PlannedRoute::from(plan),
        Err(RouteError::ModelNotFound) => {
            let available = available_models(&state, &auth_context.allowed_models).await;
            return Err(Failure::from_error(
                telemetry,
                ProxyError::from(AppError::model_not_found(
                    format!("模型 `{}` 不存在或未发布", parsed.alias()),
                    available,
                )),
            ));
        }
        Err(RouteError::NoRoute) => {
            return Err(Failure::from_error(telemetry, model_unavailable()));
        }
        Err(RouteError::DatabaseUnavailable) => {
            return Err(Failure::from_error(
                telemetry,
                ProxyError::from(AppError::database_unavailable()),
            ));
        }
    };
    if plan.source == super::route::RouteSource::Catalog {
        telemetry.model_group = Some(plan.model_alias.clone());
    }
    telemetry.catalog_version = plan.catalog_version.clone();
    let Some(first_target) = plan.targets.first() else {
        return Err(Failure::from_error(telemetry, model_unavailable()));
    };
    // 供预算 429 等错误响应携带 x-gw-resolved-model/model-group/catalog-version。
    telemetry.with_target(first_target);

    // 7. 预算预检（不调上游；spend/policy 失败关闭）。
    let estimate = budget::estimate(
        first_target,
        parsed.prompt_chars(),
        parsed.requested_max_tokens(),
        settings.usd_cny_rate,
    );
    if let Err(error) = budget::check(&state, auth_context.user_id, &estimate).await {
        return Err(Failure::from_error(telemetry, ProxyError::from(error)));
    }

    // 8. 上游尝试（≤2 次；候选/账号降级）。
    let prepared = parsed.prepared();
    let stream_requested = parsed.is_stream();
    let execution = if stream_requested {
        executor::execute_stream(&state, &settings, &plan.targets, &prepared)
            .await
            .map(ExecutionOutcome::Stream)
    } else {
        executor::execute_json(&state, &settings, &plan.targets, &prepared)
            .await
            .map(ExecutionOutcome::Json)
    };
    let success = match execution {
        Ok(success) => success,
        Err(failure) => return Err(execution_failure(telemetry, failure)),
    };

    match success {
        ExecutionOutcome::Json(success) => {
            let target = success.target.clone();
            telemetry.with_target(&target);
            telemetry.fallback = success.fallback;
            telemetry.attempt_count = success.attempt_count;
            finish_json(settings, telemetry, parsed, &target, success).await
        }
        ExecutionOutcome::Stream(success) => {
            let target = success.target.clone();
            telemetry.with_target(&target);
            telemetry.fallback = success.fallback;
            telemetry.attempt_count = success.attempt_count;
            finish_stream(settings, telemetry, parsed, &target, success)
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum ParsedRequest {
    Chat(ChatRequest),
    Messages(MessagesRequest),
    Responses(ResponsesRequest),
    Gemini(GeminiRequest),
}

impl ParsedRequest {
    fn is_stream(&self) -> bool {
        match self {
            Self::Chat(request) => request.is_stream,
            Self::Messages(request) => request.is_stream,
            Self::Responses(request) => request.is_stream,
            Self::Gemini(request) => request.is_stream,
        }
    }

    fn alias(&self) -> &str {
        match self {
            Self::Chat(request) => &request.alias,
            Self::Messages(request) => &request.alias,
            Self::Responses(request) => &request.alias,
            Self::Gemini(request) => &request.alias,
        }
    }

    fn adapter(&self) -> &'static str {
        match self {
            Self::Chat(_) => chat::ADAPTER_ID,
            Self::Messages(_) => messages::ADAPTER_ID,
            Self::Responses(_) => responses::ADAPTER_ID,
            Self::Gemini(_) => gemini::ADAPTER_ID,
        }
    }

    fn prompt_chars(&self) -> usize {
        match self {
            Self::Chat(request) => request.prompt_chars,
            Self::Messages(request) => request.prompt_chars,
            Self::Responses(request) => request.prompt_chars,
            Self::Gemini(request) => request.prompt_chars,
        }
    }

    fn requested_max_tokens(&self) -> Option<i64> {
        match self {
            Self::Chat(request) => request.max_tokens,
            Self::Messages(request) => Some(request.max_tokens),
            Self::Responses(request) => request.max_tokens,
            Self::Gemini(request) => request.max_tokens,
        }
    }

    fn prepared(&self) -> PreparedBody<'_> {
        match self {
            Self::Chat(request) => PreparedBody::Chat(request),
            Self::Messages(request) => PreparedBody::Messages(request),
            Self::Responses(request) => PreparedBody::Responses(request),
            Self::Gemini(request) => PreparedBody::Gemini(request),
        }
    }

    fn degraded(&self) -> &[&'static str] {
        match self {
            Self::Chat(_) => &[],
            Self::Messages(request) => &request.degraded,
            Self::Responses(request) => &request.degraded,
            Self::Gemini(request) => &request.degraded,
        }
    }
}

enum ExecutionOutcome {
    Json(executor::JsonSuccess),
    Stream(executor::StreamSuccess),
}

#[allow(clippy::result_large_err)]
async fn finish_json(
    settings: ProxySettings,
    telemetry: Telemetry,
    parsed: ParsedRequest,
    target: &PlannedTarget,
    success: executor::JsonSuccess,
) -> Result<Response, Failure> {
    let (response_body, usage, output_chars, decoded_degraded) = match &parsed {
        ParsedRequest::Chat(request) => {
            match chat::decode_non_stream(&success.upstream.body, &request.alias) {
                Ok((body, usage, output_chars)) => (body, usage, output_chars, Vec::new()),
                Err(error) => {
                    return Err(Failure::from_error(telemetry, ProxyError::from(error)));
                }
            }
        }
        ParsedRequest::Messages(request) => {
            match messages::decode_non_stream(
                &success.upstream.body,
                request,
                &telemetry.request_id,
            ) {
                Ok(decoded) => (
                    decoded.body,
                    decoded.usage,
                    decoded.output_chars,
                    decoded.degraded,
                ),
                Err(error) => {
                    return Err(Failure::from_error(telemetry, ProxyError::from(error)));
                }
            }
        }
        ParsedRequest::Responses(request) => {
            match responses::decode_non_stream(
                &success.upstream.body,
                request,
                &telemetry.request_id,
            ) {
                Ok(decoded) => (
                    decoded.body,
                    decoded.usage,
                    decoded.output_chars,
                    decoded.degraded,
                ),
                Err(error) => {
                    return Err(Failure::from_error(telemetry, ProxyError::from(error)));
                }
            }
        }
        ParsedRequest::Gemini(request) => {
            match gemini::decode_non_stream(&success.upstream.body, request, &telemetry.request_id)
            {
                Ok(decoded) => (
                    decoded.body,
                    decoded.usage,
                    decoded.output_chars,
                    decoded.degraded,
                ),
                Err(error) => {
                    return Err(Failure::from_error(telemetry, ProxyError::from(error)));
                }
            }
        }
    };
    let input_tokens = usage
        .input_tokens
        .unwrap_or_else(|| estimate_tokens(parsed.prompt_chars()));
    let output_tokens = usage
        .output_tokens
        .unwrap_or_else(|| estimate_tokens(output_chars));
    let usage_source = if usage.input_tokens.is_some() || usage.output_tokens.is_some() {
        UsageSource::Upstream
    } else {
        UsageSource::Estimated
    };
    let mut degraded = parsed.degraded().to_vec();
    degraded.extend(decoded_degraded);
    let request_id = telemetry.request_id.clone();
    let model_group = telemetry.model_group.clone();
    let catalog_version = telemetry.catalog_version.clone();
    let headers = success_headers(
        &settings,
        &request_id,
        target,
        success.fallback,
        parsed.adapter(),
        &degraded,
        false,
        model_group.as_deref(),
        catalog_version.as_deref(),
    );
    telemetry.finalize(Outcome {
        status: UsageStatus::Success,
        http_status: 200,
        error_code: None,
        input_tokens: Some(input_tokens),
        output_tokens: Some(output_tokens),
        usage_source,
        ttft_ms: None,
        response_size: Some(response_body.len() as i64),
    });
    Ok(with_headers(response_body.into_response(), headers))
}

#[allow(clippy::result_large_err)]
fn finish_stream(
    settings: ProxySettings,
    telemetry: Telemetry,
    parsed: ParsedRequest,
    target: &PlannedTarget,
    success: executor::StreamSuccess,
) -> Result<Response, Failure> {
    let request_id = telemetry.request_id.clone();
    let input_estimate = estimate_tokens(parsed.prompt_chars());
    let model_group = telemetry.model_group.clone();
    let catalog_version = telemetry.catalog_version.clone();
    let headers = success_headers(
        &settings,
        &request_id,
        target,
        success.fallback,
        parsed.adapter(),
        parsed.degraded(),
        true,
        model_group.as_deref(),
        catalog_version.as_deref(),
    );
    let stream_body = match &parsed {
        ParsedRequest::Chat(_) => {
            stream::chat_sse_body(success.upstream, telemetry, input_estimate, settings)
        }
        ParsedRequest::Messages(request) => {
            stream::messages_sse_body(success.upstream, telemetry, request, &request_id, settings)
        }
        ParsedRequest::Responses(request) => {
            stream::responses_sse_body(success.upstream, telemetry, request, &request_id, settings)
        }
        ParsedRequest::Gemini(request) => {
            stream::gemini_sse_body(success.upstream, telemetry, request, &request_id, settings)
        }
    };
    Ok(with_headers(stream_body.into_response(), headers))
}

fn execution_failure(telemetry: Telemetry, failure: ExecutionFailure) -> Failure {
    let telemetry = match failure.target {
        Some(target) => {
            let mut telemetry = telemetry;
            telemetry.with_target(&target);
            telemetry.fallback = failure.fallback;
            telemetry.attempt_count = failure.attempt_count;
            telemetry
        }
        None => telemetry,
    };
    Failure {
        error: failure.error,
        telemetry,
        outcome: Outcome {
            status: failure.usage_status,
            http_status: 500,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            usage_source: UsageSource::Missing,
            ttft_ms: None,
            response_size: None,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn success_headers(
    settings: &ProxySettings,
    request_id: &str,
    target: &PlannedTarget,
    fallback: bool,
    adapter: &'static str,
    degraded: &[&'static str],
    is_stream: bool,
    model_group: Option<&str>,
    catalog_version: Option<&str>,
) -> Vec<(HeaderName, HeaderValue)> {
    let mut headers = Vec::new();
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.push((HeaderName::from_static("x-request-id"), value));
    }
    headers.push((header::CACHE_CONTROL, HeaderValue::from_static("no-store")));
    if let Ok(value) = HeaderValue::from_str(&target.upstream_model) {
        headers.push((HeaderName::from_static("x-gw-resolved-model"), value));
    }
    if fallback {
        headers.push((
            HeaderName::from_static("x-gw-fallback"),
            HeaderValue::from_static("true"),
        ));
    }
    headers.push((
        HeaderName::from_static("x-gw-adapter"),
        HeaderValue::from_static(adapter),
    ));
    if !degraded.is_empty()
        && let Ok(value) = HeaderValue::from_str(&degraded.join(","))
    {
        headers.push((HeaderName::from_static("x-gw-degraded"), value));
    }
    if settings.expose_provider_header
        && let Ok(value) = HeaderValue::from_str(&target.provider_name)
    {
        headers.push((HeaderName::from_static("x-gw-provider"), value));
    }
    if let Some(model_group) = model_group
        && let Ok(value) = HeaderValue::from_str(model_group)
    {
        headers.push((HeaderName::from_static("x-gw-model-group"), value));
    }
    if let Some(catalog_version) = catalog_version
        && let Ok(value) = HeaderValue::from_str(catalog_version)
    {
        headers.push((HeaderName::from_static("x-gw-catalog-version"), value));
    }
    if is_stream {
        headers.push((
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        ));
        headers.push((
            HeaderName::from_static("x-accel-buffering"),
            HeaderValue::from_static("no"),
        ));
    } else {
        headers.push((
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        ));
    }
    headers
}

fn with_headers(mut response: Response, headers: Vec<(HeaderName, HeaderValue)>) -> Response {
    for (name, value) in headers {
        response.headers_mut().insert(name, value);
    }
    response
}

struct Failure {
    error: ProxyError,
    telemetry: Telemetry,
    outcome: Outcome,
}

impl Failure {
    fn from_error(telemetry: Telemetry, error: ProxyError) -> Self {
        let outcome = outcome_for_error(&error);
        Self {
            error,
            telemetry,
            outcome,
        }
    }

    fn render(self, expose_provider: bool) -> Response {
        let status = self.error.status();
        let code = self.error.code();
        let request_id = Some(self.telemetry.request_id.clone());
        let resolved_model = self
            .telemetry
            .resolved
            .as_ref()
            .map(|resolved| resolved.upstream_model.clone());
        let fallback = self.telemetry.fallback;
        let provider = if expose_provider {
            self.telemetry
                .resolved
                .as_ref()
                .map(|resolved| resolved.provider_name.clone())
        } else {
            None
        };
        let model_group = self.telemetry.model_group.clone();
        let catalog_version = self.telemetry.catalog_version.clone();
        let mut outcome = self.outcome;
        outcome.http_status = status.as_u16();
        outcome.error_code = Some(code.to_string());
        self.telemetry.finalize(outcome);
        DataPlaneError::new(self.error)
            .with_route(request_id, resolved_model, fallback, provider)
            .with_catalog(model_group, catalog_version)
            .into_response()
    }
}

fn outcome_for_error(error: &ProxyError) -> Outcome {
    let code = error.code();
    let status = error.status();
    let usage_status = match code {
        "upstream_timeout" => UsageStatus::Timeout,
        code if code.starts_with("upstream") => UsageStatus::UpstreamError,
        "internal_error"
        | "credential_unavailable"
        | "database_unavailable"
        | "model_unavailable" => UsageStatus::GatewayError,
        _ => UsageStatus::Rejected,
    };
    Outcome {
        status: usage_status,
        http_status: status.as_u16(),
        error_code: Some(code.to_string()),
        input_tokens: None,
        output_tokens: None,
        usage_source: UsageSource::Missing,
        ttft_ms: None,
        response_size: None,
    }
}

#[allow(clippy::result_large_err)]
async fn available_models(state: &AppState, allowed_models: &[String]) -> Vec<String> {
    match routing_service::visible_models(state, allowed_models).await {
        Ok(models) => models.into_iter().take(50).map(|model| model.id).collect(),
        Err(_) => Vec::new(),
    }
}

fn content_type_ok(headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    value.to_ascii_lowercase().contains("application/json")
}

enum BodyReadError {
    TooLarge,
    Read,
}

async fn read_body(request: Request, limit: usize) -> Result<Bytes, BodyReadError> {
    let mut stream = request.into_body().into_data_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| BodyReadError::Read)?;
        if body.len() + chunk.len() > limit {
            return Err(BodyReadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_requires_json() {
        let mut headers = HeaderMap::new();
        assert!(!content_type_ok(&headers));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        assert!(content_type_ok(&headers));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!content_type_ok(&headers));
    }

    #[test]
    fn error_outcome_classification() {
        let rejected = ProxyError::from(AppError::Forbidden {
            code: "model_not_allowed",
            detail: "x".to_string(),
        });
        assert_eq!(outcome_for_error(&rejected).status, UsageStatus::Rejected);
        let gateway = ProxyError::from(AppError::database_unavailable());
        assert_eq!(
            outcome_for_error(&gateway).status,
            UsageStatus::GatewayError
        );
        let timeout = ProxyError::from(AppError::gateway_timeout("upstream_timeout", "x", None));
        assert_eq!(outcome_for_error(&timeout).status, UsageStatus::Timeout);
    }
}
