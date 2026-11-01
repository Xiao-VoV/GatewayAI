//! catalog HTTP handlers：客户端 `/v1/models`/`/v1/models/{id}`（发布投影）+
//! v0.1.x 控制面模型组/部署/发布（16 端点）。
//!
//! 数据面鉴权统一走 `access::authenticate_headers`；目录投影走 catalog 发布（v0.1.x）；
//! 控制面走 identity `AdminSession` + CSRF。catalog 不写 usage；数据面仅写 observability 请求日志。

use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    time::Instant,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{
        ConnectInfo, DefaultBodyLimit, Path, Query, Request, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::Utc;
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::modules::{
    access::{
        model::AuthContext,
        service::{self as access_service, AuthError},
    },
    identity::middleware::{AdminSession, require_csrf},
    observability::{
        model::{ProtocolKind, RequestLogEvent},
        service as observability_service,
    },
};
use crate::platform::{error::AppError, http::request_id_from_headers, state::AppState};

use super::{
    model::{
        CatalogPreviewQuery, CatalogSubject, ConfirmMergeSuggestionRequest,
        DeploymentCreateRequest, DeploymentListQuery, DeploymentUpdateRequest, MAX_BODY_BYTES,
        ModelGroupCreateRequest, ModelGroupListQuery, ModelGroupUpdateRequest,
        PublicationCreateRequest, PublicationListQuery, PublicationUpdateRequest,
        RejectMergeSuggestionRequest, SuggestionListQuery, ToolDefaultCreateRequest,
        ToolDefaultListQuery, ToolDefaultUpdateRequest,
    },
    service,
};

// ===========================================================================
// 路由
// ===========================================================================

pub fn client_router() -> Router<AppState> {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/models/{model_id}", get(get_model))
        .route("/v1/model-defaults", get(list_model_defaults))
}

/// 控制面：模型组 / 部署 / 发布（16 端点）。
/// 聚合建议/工具默认/员工预览未实现，暂不注册（不伪造 200）。
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/model-groups",
            get(list_model_groups).post(create_model_group),
        )
        .route(
            "/api/admin/model-groups/{model_group_id}",
            get(get_model_group)
                .patch(update_model_group)
                .delete(delete_model_group),
        )
        .route(
            "/api/admin/model-groups/{model_group_id}/route-preview",
            get(preview_model_group_routes),
        )
        .route(
            "/api/admin/deployments",
            get(list_deployments).post(create_deployment),
        )
        .route(
            "/api/admin/deployments/{deployment_id}",
            get(get_deployment)
                .patch(update_deployment)
                .delete(delete_deployment),
        )
        .route(
            "/api/admin/publications",
            get(list_publications).post(create_publication),
        )
        .route(
            "/api/admin/publications/{publication_id}",
            get(get_publication)
                .patch(update_publication)
                .delete(delete_publication),
        )
        .route(
            "/api/admin/tool-model-defaults",
            get(list_tool_model_defaults).post(create_tool_model_default),
        )
        .route("/api/admin/catalog/preview", get(preview_admin_catalog))
        .route(
            "/api/admin/model-group-suggestions",
            get(list_model_group_suggestions),
        )
        .route(
            "/api/admin/model-group-suggestions/{suggestion_key}/confirm",
            axum::routing::post(confirm_model_group_suggestion),
        )
        .route(
            "/api/admin/model-group-suggestions/{suggestion_key}/reject",
            axum::routing::post(reject_model_group_suggestion),
        )
        .route(
            "/api/admin/tool-model-defaults/{tool_default_id}",
            axum::routing::patch(update_tool_model_default).delete(delete_tool_model_default),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

// ===========================================================================
// 控制面 handlers：模型组
// ===========================================================================

async fn list_model_groups(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<ModelGroupListQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_model_groups(&state, query).await?;
    Ok(no_store_json(page))
}

async fn create_model_group(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<ModelGroupCreateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let group = service::create_model_group(&state, request).await?;
    Ok(created_json(group))
}

async fn get_model_group(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(model_group_id): Path<String>,
) -> Result<Response, AppError> {
    let model_group_id = parse_model_group_id(&model_group_id)?;
    let group = service::get_model_group(&state, model_group_id).await?;
    Ok(no_store_json(group))
}

async fn update_model_group(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(model_group_id): Path<String>,
    payload: Result<Json<ModelGroupUpdateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let model_group_id = parse_model_group_id(&model_group_id)?;
    let request = json_body(payload)?;
    let group = service::update_model_group(&state, model_group_id, request).await?;
    Ok(no_store_json(group))
}

async fn delete_model_group(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(model_group_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let model_group_id = parse_model_group_id(&model_group_id)?;
    service::archive_model_group(&state, model_group_id).await?;
    Ok(no_content())
}

async fn preview_model_group_routes(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(model_group_id): Path<String>,
) -> Result<Response, AppError> {
    let model_group_id = parse_model_group_id(&model_group_id)?;
    let preview = service::preview_model_group_routes(&state, model_group_id).await?;
    Ok(no_store_json(preview))
}

// ===========================================================================
// 控制面 handlers：部署
// ===========================================================================

async fn list_deployments(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<DeploymentListQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_deployments(&state, query).await?;
    Ok(no_store_json(page))
}

async fn create_deployment(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<DeploymentCreateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let deployment = service::create_deployment(&state, request).await?;
    Ok(created_json(deployment))
}

async fn get_deployment(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(deployment_id): Path<String>,
) -> Result<Response, AppError> {
    let deployment_id = parse_deployment_id(&deployment_id)?;
    let deployment = service::get_deployment(&state, deployment_id).await?;
    Ok(no_store_json(deployment))
}

async fn update_deployment(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(deployment_id): Path<String>,
    payload: Result<Json<DeploymentUpdateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let deployment_id = parse_deployment_id(&deployment_id)?;
    let request = json_body(payload)?;
    let deployment = service::update_deployment(&state, deployment_id, request).await?;
    Ok(no_store_json(deployment))
}

async fn delete_deployment(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(deployment_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let deployment_id = parse_deployment_id(&deployment_id)?;
    service::delete_deployment(&state, deployment_id).await?;
    Ok(no_content())
}

// ===========================================================================
// 控制面 handlers：发布
// ===========================================================================

async fn list_publications(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<PublicationListQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_publications(&state, query).await?;
    Ok(no_store_json(page))
}

async fn create_publication(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<PublicationCreateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let publication = service::create_publication(&state, request, session.user.id).await?;
    Ok(created_json(publication))
}

async fn get_publication(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(publication_id): Path<String>,
) -> Result<Response, AppError> {
    let publication_id = parse_publication_id(&publication_id)?;
    let publication = service::get_publication(&state, publication_id).await?;
    Ok(no_store_json(publication))
}

async fn update_publication(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(publication_id): Path<String>,
    payload: Result<Json<PublicationUpdateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let publication_id = parse_publication_id(&publication_id)?;
    let request = json_body(payload)?;
    let publication = service::update_publication(&state, publication_id, request).await?;
    Ok(no_store_json(publication))
}

async fn delete_publication(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(publication_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let publication_id = parse_publication_id(&publication_id)?;
    service::delete_publication(&state, publication_id).await?;
    Ok(no_content())
}

// ===========================================================================
// 控制面 handlers：工具默认模型
// ===========================================================================

async fn list_tool_model_defaults(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<ToolDefaultListQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_tool_model_defaults(&state, query).await?;
    Ok(no_store_json(page))
}

async fn create_tool_model_default(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<ToolDefaultCreateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let created = service::create_tool_model_default(&state, request).await?;
    Ok(created_json(created))
}

async fn update_tool_model_default(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(tool_default_id): Path<String>,
    payload: Result<Json<ToolDefaultUpdateRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let tool_default_id = parse_tool_default_id(&tool_default_id)?;
    let request = json_body(payload)?;
    let updated = service::update_tool_model_default(&state, tool_default_id, request).await?;
    Ok(no_store_json(updated))
}

async fn delete_tool_model_default(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(tool_default_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let tool_default_id = parse_tool_default_id(&tool_default_id)?;
    service::delete_tool_model_default(&state, tool_default_id).await?;
    Ok(no_content())
}

// ===========================================================================
// 控制面 handler：员工目录预览（只读诊断，无 CSRF）
// ===========================================================================

async fn preview_admin_catalog(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<CatalogPreviewQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let preview = service::admin_catalog_preview(&state, query).await?;
    Ok(no_store_json(preview))
}

// ===========================================================================
// 控制面 handlers：聚合建议（confirm/reject 需 CSRF）
// ===========================================================================

async fn list_model_group_suggestions(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<SuggestionListQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_model_group_suggestions(&state, query).await?;
    Ok(no_store_json(page))
}

async fn confirm_model_group_suggestion(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(suggestion_key): Path<String>,
    payload: Result<Json<ConfirmMergeSuggestionRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let confirmation =
        service::confirm_model_group_suggestion(&state, &suggestion_key, request, session.user.id)
            .await?;
    Ok(no_store_json(confirmation))
}

async fn reject_model_group_suggestion(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(suggestion_key): Path<String>,
    payload: Result<Json<RejectMergeSuggestionRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let rejection =
        service::reject_model_group_suggestion(&state, &suggestion_key, request, session.user.id)
            .await?;
    Ok(no_store_json(rejection))
}

// ===========================================================================
// 控制面 helpers
// ===========================================================================

fn parse_model_group_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("model_group_not_found", "模型组不存在"))
}

fn parse_deployment_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("deployment_not_found", "部署不存在"))
}

fn parse_publication_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("publication_not_found", "发布不存在"))
}

fn parse_tool_default_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw)
        .map_err(|_| AppError::not_found("tool_model_default_not_found", "工具默认模型不存在"))
}

fn parse_query<T: DeserializeOwned>(
    query: Result<Query<T>, QueryRejection>,
) -> Result<T, AppError> {
    query.map(|Query(value)| value).map_err(|rejection| {
        AppError::validation_field(
            "query",
            format!("查询参数不合法: {}", rejection.body_text()),
        )
    })
}

fn json_body<T: DeserializeOwned>(payload: Result<Json<T>, JsonRejection>) -> Result<T, AppError> {
    payload.map(|Json(value)| value).map_err(|rejection| {
        let mut errors = BTreeMap::new();
        errors.insert(
            "body".to_string(),
            format!("请求体不合法: {}", rejection.body_text()),
        );
        AppError::validation(errors)
    })
}

fn no_store_json<T: Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    add_no_store(&mut response);
    response
}

fn created_json<T: Serialize>(value: T) -> Response {
    let mut response = (StatusCode::CREATED, Json(value)).into_response();
    add_no_store(&mut response);
    response
}

fn no_content() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    add_no_store(&mut response);
    response
}

fn add_no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

// ===========================================================================
// 数据面：发布投影（v0.1.x）
// ===========================================================================

const CATALOG_VERSION_HEADER: &str = "x-gw-catalog-version";

struct RequestLogContext {
    request_id: String,
    method: String,
    path: &'static str,
    started_at: Instant,
    created_at: chrono::DateTime<Utc>,
    client_ip: Option<String>,
    user_agent: Option<String>,
}

impl RequestLogContext {
    fn from_request(request: &Request, path: &'static str, trust_proxy_headers: bool) -> Self {
        let headers = request.headers();
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect| connect.0);
        let request_id =
            request_id_from_headers(headers).unwrap_or_else(|| Uuid::new_v4().to_string());
        Self {
            request_id,
            method: "GET".to_string(),
            path,
            started_at: Instant::now(),
            created_at: Utc::now(),
            client_ip: client_ip(peer, headers, trust_proxy_headers),
            user_agent: user_agent(headers),
        }
    }

    fn record(
        &self,
        state: &AppState,
        auth: Option<&AuthContext>,
        status_code: u16,
        response_size: Option<usize>,
        error_code: Option<&str>,
    ) {
        let event = RequestLogEvent {
            request_id: self.request_id.clone(),
            trace_id: Some(self.request_id.clone()),
            method: self.method.clone(),
            path: self.path.to_string(),
            protocol: ProtocolKind::Models,
            upstream_protocol: None,
            adapter: None,
            user_id: auth.map(|auth| auth.user_id),
            device_id: auth.map(|auth| auth.device_id),
            key_id: auth.map(|auth| auth.key_id),
            key_prefix: auth.and_then(|auth| auth.key_prefix.clone()),
            model_alias: None,
            upstream_model: None,
            provider_id: None,
            provider: None,
            is_stream: false,
            fallback: false,
            attempt_count: 1,
            status_code: i32::from(status_code),
            latency_ms: self.started_at.elapsed().as_millis() as i64,
            ttft_ms: None,
            response_size: response_size.map(|size| size as i64),
            error_code: error_code.map(ToString::to_string),
            client_ip: self.client_ip.clone(),
            user_agent: self.user_agent.clone(),
            created_at: self.created_at,
        };
        observability_service::record_request_log_detached(state, event);
    }
}

fn subject_from_auth(auth: &AuthContext) -> CatalogSubject {
    CatalogSubject {
        user_id: auth.user_id,
        device_id: Some(auth.device_id),
        user_group_ids: auth.user_group_ids.clone(),
        key_allowed_models: auth.allowed_models.clone(),
    }
}

/// 序列化目录响应：显式 bytes 以记录 response_size，并写 `x-gw-catalog-version`。
fn catalog_response<T: Serialize>(value: &T, version: &str) -> Result<(Response, usize), AppError> {
    let bytes = serde_json::to_vec(value).map_err(AppError::internal)?;
    let response_size = bytes.len();
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .expect("valid catalog response");
    if let Ok(value) = HeaderValue::from_str(version) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(CATALOG_VERSION_HEADER), value);
    }
    Ok((response, response_size))
}

async fn list_models(State(state): State<AppState>, request: Request) -> Response {
    let log =
        RequestLogContext::from_request(&request, "/v1/models", state.config.trust_proxy_headers);
    let headers = request.headers().clone();
    let auth = match access_service::authenticate_headers(&state, &headers).await {
        Ok(auth) => auth,
        Err(error) => return auth_failure(&state, &log, error),
    };
    let subject = subject_from_auth(&auth);
    let catalog = match service::client_catalog(&state, &subject).await {
        Ok(catalog) => catalog,
        Err(error) => return catalog_error(&state, &log, Some(&auth), error),
    };
    let version = catalog.catalog_version.clone();
    let (response, response_size) = match catalog_response(&catalog, &version) {
        Ok(result) => result,
        Err(error) => return catalog_error(&state, &log, Some(&auth), error),
    };
    log.record(&state, Some(&auth), 200, Some(response_size), None);
    response
}

async fn list_model_defaults(State(state): State<AppState>, request: Request) -> Response {
    let log = RequestLogContext::from_request(
        &request,
        "/v1/model-defaults",
        state.config.trust_proxy_headers,
    );
    let headers = request.headers().clone();
    let auth = match access_service::authenticate_headers(&state, &headers).await {
        Ok(auth) => auth,
        Err(error) => return auth_failure(&state, &log, error),
    };
    let subject = subject_from_auth(&auth);
    let defaults = match service::client_model_defaults(&state, &subject).await {
        Ok(defaults) => defaults,
        Err(error) => return catalog_error(&state, &log, Some(&auth), error),
    };
    let version = defaults.catalog_version.clone();
    let (response, response_size) = match catalog_response(&defaults, &version) {
        Ok(result) => result,
        Err(error) => return catalog_error(&state, &log, Some(&auth), error),
    };
    log.record(&state, Some(&auth), 200, Some(response_size), None);
    response
}

async fn get_model(
    State(state): State<AppState>,
    Path(model_id): Path<String>,
    request: Request,
) -> Response {
    let log = RequestLogContext::from_request(
        &request,
        "/v1/models/{id}",
        state.config.trust_proxy_headers,
    );
    let headers = request.headers().clone();
    let auth = match access_service::authenticate_headers(&state, &headers).await {
        Ok(auth) => auth,
        Err(error) => return auth_failure(&state, &log, error),
    };
    let subject = subject_from_auth(&auth);
    let detail = match service::client_model_detail(&state, &subject, &model_id).await {
        Ok(detail) => detail,
        Err(error) => return catalog_error(&state, &log, Some(&auth), error),
    };
    let Some(detail) = detail else {
        let error = AppError::not_found("model_not_found", "模型不存在或未发布给当前用户");
        let response = error.into_response();
        log.record(&state, Some(&auth), 404, None, Some("model_not_found"));
        return response;
    };
    let version = detail.catalog_version.clone();
    let (response, response_size) = match catalog_response(&detail, &version) {
        Ok(result) => result,
        Err(error) => return catalog_error(&state, &log, Some(&auth), error),
    };
    log.record(&state, Some(&auth), 200, Some(response_size), None);
    response
}

fn catalog_error(
    state: &AppState,
    log: &RequestLogContext,
    auth: Option<&AuthContext>,
    error: AppError,
) -> Response {
    let response = error.into_response();
    let status = response.status();
    let code = match status {
        StatusCode::SERVICE_UNAVAILABLE => "database_unavailable",
        _ => "internal_error",
    };
    log.record(state, auth, status.as_u16(), None, Some(code));
    response
}

fn auth_failure(state: &AppState, log: &RequestLogContext, error: AuthError) -> Response {
    let code = error.code();
    let app_error = access_service::auth_error_to_app_error(error);
    let response = app_error.into_response();
    log.record(state, None, response.status().as_u16(), None, Some(code));
    response
}

fn client_ip(
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    trust_proxy_headers: bool,
) -> Option<String> {
    // 与 proxy 共用约定：仅显式信任代理头时才采信 XFF。
    if trust_proxy_headers
        && let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
    {
        return Some(ip.to_string());
    }
    peer.map(|peer| peer.ip().to_string())
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect())
}
