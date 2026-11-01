//! policy HTTP handlers：管理台预算 4 端点。
//!
//! 契约：`contracts/policy.yaml` v0.1.0。

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
    model::{MAX_BODY_BYTES, PageQuery, UpsertBudgetRequest},
    service,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/budgets", get(list_budgets))
        .route(
            "/api/admin/budgets/{user_id}",
            get(get_budget).put(upsert_budget).delete(delete_budget),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

async fn list_budgets(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_budgets(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn get_budget(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(user_id): Path<String>,
) -> Result<Response, AppError> {
    let user_id = parse_user_id(&user_id)?;
    let budget = service::get_budget(&state, user_id).await?;
    Ok(no_store_json(budget))
}

async fn upsert_budget(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    payload: Result<Json<UpsertBudgetRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let user_id = parse_user_id(&user_id)?;
    let request = json_body(payload)?;
    let budget = service::upsert_budget(&state, user_id, request).await?;
    Ok(no_store_json(budget))
}

async fn delete_budget(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let user_id = parse_user_id(&user_id)?;
    service::delete_budget(&state, user_id).await?;
    Ok(no_content())
}

fn parse_user_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("user_not_found", "员工用户不存在"))
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
