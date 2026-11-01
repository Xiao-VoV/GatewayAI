//! metering 管理台只读查询：`GET /api/admin/overview`、`GET /api/admin/usage`。
//!
//! 设计见 `设计文档（暂未开源）` §3；S5 查询直接聚合
//! `usage_records`（明细直查，rollup 优化留后续），今日/历史同一数据源保证误差 0。
//! 管理台只读，Session 鉴权、无 CSRF；所有响应 no-store。

use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::de::DeserializeOwned;

use crate::modules::{access::service as access_service, identity::middleware::AdminSession};
use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{ClientUsageQuery, OverviewQuery, UsageQuery},
    service,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/overview", get(overview))
        .route("/api/admin/usage", get(usage))
}

/// 客户端只读用量：设备 token + 虚拟 Key；用户由鉴权上下文决定。
pub fn client_router() -> Router<AppState> {
    Router::new().route("/api/client/usage", get(client_usage))
}

async fn overview(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<OverviewQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let response = service::overview(&state, &query).await?;
    Ok(no_store_json(response))
}

async fn usage(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<UsageQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let response = service::usage(&state, &query).await?;
    Ok(no_store_json(response))
}

async fn client_usage(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    query: Result<Query<ClientUsageQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let auth = access_service::authenticate_headers(&state, &headers)
        .await
        .map_err(access_service::auth_error_to_app_error)?;
    let query = parse_query(query)?;
    let days = query.days.unwrap_or(7);
    let response = service::client_overview(&state, auth.user_id, days).await?;
    Ok(no_store_json(response))
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

fn no_store_json<T: serde::Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
