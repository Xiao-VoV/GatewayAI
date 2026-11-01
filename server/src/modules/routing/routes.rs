//! routing HTTP handlers：管理台模型别名 6 端点。
//!
//! 契约：`contracts/routing.yaml` v0.1.0。

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::modules::identity::middleware::{AdminSession, require_csrf};
use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{CreateModelAliasRequest, MAX_BODY_BYTES, PageQuery, UpdateModelAliasRequest},
    service,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/model-aliases",
            get(list_model_aliases).post(create_model_alias),
        )
        .route(
            "/api/admin/model-aliases/{alias_id}",
            get(get_model_alias)
                .patch(update_model_alias)
                .delete(delete_model_alias),
        )
        .route(
            "/api/admin/model-aliases/{alias_id}/preview",
            get(preview_model_alias),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

async fn list_model_aliases(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_model_aliases(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn create_model_alias(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<CreateModelAliasRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let alias = service::create_model_alias(&state, request).await?;
    let mut response = (StatusCode::CREATED, Json(alias)).into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn get_model_alias(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(alias_id): Path<String>,
) -> Result<Response, AppError> {
    let alias_id = parse_alias_id(&alias_id)?;
    let alias = service::get_model_alias(&state, alias_id).await?;
    Ok(no_store_json(alias))
}

async fn update_model_alias(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(alias_id): Path<String>,
    payload: Result<Json<UpdateModelAliasRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let alias_id = parse_alias_id(&alias_id)?;
    let request = json_body(payload)?;
    let alias = service::update_model_alias(&state, alias_id, request).await?;
    Ok(no_store_json(alias))
}

async fn delete_model_alias(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(alias_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let alias_id = parse_alias_id(&alias_id)?;
    service::delete_model_alias(&state, alias_id).await?;
    Ok(no_content())
}

async fn preview_model_alias(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(alias_id): Path<String>,
) -> Result<Response, AppError> {
    let alias_id = parse_alias_id(&alias_id)?;
    let preview = service::preview_model_alias(&state, alias_id).await?;
    Ok(no_store_json(preview))
}

fn parse_alias_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("model_alias_not_found", "模型别名不存在"))
}

fn parse_query(query: Result<Query<PageQuery>, QueryRejection>) -> Result<PageQuery, AppError> {
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
