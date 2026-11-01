//! providers HTTP handlers 与路由装配。契约见 `接口设计.md` §3–§6。
//!
//! 顺序：鉴权（401）→ CSRF（403）→ JSON/字段（400）→ DB（503）→ 存在性（404）→ 冲突（409）。

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::modules::identity::middleware::{AdminSession, require_csrf};
use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{
        CreateAccountRequest, CreateModelRequest, CreateProviderRequest, MAX_BODY_BYTES,
        PatchAccountRequest, PatchModelRequest, PatchProviderRequest, TestRequest,
    },
    service,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListProvidersQuery {
    status: Option<String>,
    kind: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/providers",
            get(list_providers).post(create_provider),
        )
        .route(
            "/api/admin/providers/{provider_id}",
            get(get_provider)
                .patch(update_provider)
                .delete(delete_provider),
        )
        .route(
            "/api/admin/providers/{provider_id}/test",
            post(test_provider),
        )
        .route(
            "/api/admin/providers/{provider_id}/accounts",
            get(list_accounts).post(create_account),
        )
        .route(
            "/api/admin/providers/{provider_id}/accounts/{account_id}",
            axum::routing::patch(update_account).delete(delete_account),
        )
        .route(
            "/api/admin/providers/{provider_id}/accounts/{account_id}/test",
            post(test_account),
        )
        .route(
            "/api/admin/providers/{provider_id}/models",
            get(list_models).post(create_model),
        )
        .route(
            "/api/admin/providers/{provider_id}/models/{model_id}",
            axum::routing::patch(update_model).delete(delete_model),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

async fn list_providers(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<ListProvidersQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_providers(
        &state,
        query.status.as_deref(),
        query.kind.as_deref(),
        query.limit,
        query.offset,
    )
    .await?;
    Ok(no_store_json(page))
}

async fn create_provider(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<CreateProviderRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let (provider, account) = service::create_provider(&state, request).await?;
    let mut response = (
        StatusCode::CREATED,
        Json(serde_json::json!({ "provider": provider, "account": account })),
    )
        .into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn get_provider(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(provider_id): Path<String>,
) -> Result<Response, AppError> {
    let provider_id = parse_provider_id(&provider_id)?;
    let detail = service::get_provider(&state, provider_id).await?;
    Ok(no_store_json(detail))
}

async fn update_provider(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    payload: Result<Json<PatchProviderRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let request = json_body(payload)?;
    let provider = service::update_provider(&state, provider_id, request).await?;
    Ok(no_store_json(provider))
}

async fn delete_provider(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    service::delete_provider(&state, provider_id).await?;
    Ok(no_content())
}

async fn test_provider(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    payload: Option<Json<TestRequest>>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let request = payload.map(|Json(value)| value).unwrap_or_default();
    let result = service::test_provider(&state, provider_id, request).await?;
    Ok(no_store_json(result))
}

async fn list_accounts(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(provider_id): Path<String>,
) -> Result<Response, AppError> {
    let provider_id = parse_provider_id(&provider_id)?;
    let accounts = service::list_accounts(&state, provider_id).await?;
    Ok(no_store_json(accounts))
}

async fn create_account(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    payload: Result<Json<CreateAccountRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let request = json_body(payload)?;
    let account = service::create_account(&state, provider_id, request).await?;
    let mut response = (StatusCode::CREATED, Json(account)).into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn update_account(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path((provider_id, account_id)): Path<(String, String)>,
    payload: Result<Json<PatchAccountRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let account_id = parse_account_id(&account_id)?;
    let request = json_body(payload)?;
    let account = service::update_account(&state, provider_id, account_id, request).await?;
    Ok(no_store_json(account))
}

async fn delete_account(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path((provider_id, account_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let account_id = parse_account_id(&account_id)?;
    service::delete_account(&state, provider_id, account_id).await?;
    Ok(no_content())
}

async fn test_account(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path((provider_id, account_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let account_id = parse_account_id(&account_id)?;
    let result = service::test_provider(
        &state,
        provider_id,
        TestRequest {
            account_id: Some(account_id),
            model: None,
        },
    )
    .await?;
    Ok(no_store_json(result))
}

async fn list_models(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(provider_id): Path<String>,
) -> Result<Response, AppError> {
    let provider_id = parse_provider_id(&provider_id)?;
    let models = service::list_models(&state, provider_id).await?;
    Ok(no_store_json(models))
}

async fn create_model(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    payload: Result<Json<CreateModelRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let request = json_body(payload)?;
    let model = service::create_model(&state, provider_id, request).await?;
    let mut response = (StatusCode::CREATED, Json(model)).into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn update_model(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path((provider_id, model_id)): Path<(String, String)>,
    payload: Result<Json<PatchModelRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let model_id = parse_model_id(&model_id)?;
    let request = json_body(payload)?;
    let model = service::update_model(&state, provider_id, model_id, request).await?;
    Ok(no_store_json(model))
}

async fn delete_model(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path((provider_id, model_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let provider_id = parse_provider_id(&provider_id)?;
    let model_id = parse_model_id(&model_id)?;
    service::delete_model(&state, provider_id, model_id).await?;
    Ok(no_content())
}

fn parse_provider_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("provider_not_found", "供应商不存在"))
}

fn parse_account_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("account_not_found", "渠道账号不存在"))
}

fn parse_model_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw)
        .map_err(|_| AppError::not_found("upstream_model_not_found", "上游模型不存在"))
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
