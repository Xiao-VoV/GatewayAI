//! observability 管理台路由：请求日志列表/详情/CSV 导出、审计列表/CSV 导出。
//!
//! 设计见 `设计文档（暂未开源）` §3/§8 与
//! `审计与导出实现设计.md`；只读 AdminSession、无 CSRF；所有响应 `Cache-Control: no-store`。
//! 审计/导出均为只读接口；审计写入只经进程内 service（`record_audit`）。

use std::net::{IpAddr, SocketAddr};

use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, Path, Query, Request, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::de::DeserializeOwned;

use crate::modules::identity::middleware::AdminSession;
use crate::platform::{error::AppError, http::request_id_from_headers, state::AppState};

use super::{
    model::{AuditListQuery, RequestListQuery},
    service,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/requests", get(requests))
        .route("/api/admin/requests/export", get(requests_export))
        .route("/api/admin/requests/{request_id}", get(request_detail))
        .route("/api/admin/audit", get(audit))
        .route("/api/admin/audit/export", get(audit_export))
}

async fn requests(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<RequestListQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_requests(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn request_detail(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(request_id): Path<String>,
) -> Result<Response, AppError> {
    let detail = service::request_detail(&state, &request_id).await?;
    Ok(no_store_json(detail))
}

async fn audit(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<AuditListQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_audits(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn requests_export(
    State(state): State<AppState>,
    session: AdminSession,
    query: Result<Query<RequestListQuery>, QueryRejection>,
    request: Request,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let actor = export_actor(&state, &session, &request);
    let export = service::export_requests_csv(&state, &query, &actor).await?;
    csv_response(export)
}

async fn audit_export(
    State(state): State<AppState>,
    session: AdminSession,
    query: Result<Query<AuditListQuery>, QueryRejection>,
    request: Request,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let actor = export_actor(&state, &session, &request);
    let export = service::export_audits_csv(&state, &query, &actor).await?;
    csv_response(export)
}

/// 导出审计事件的调用者上下文：AdminSession + peer IP（可选受信 XFF）/UA/request_id。
fn export_actor(
    state: &AppState,
    session: &AdminSession,
    request: &Request,
) -> service::ExportActor {
    let headers = request.headers();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0);
    let client_ip = if state.config.trust_proxy_headers {
        forwarded_client_ip(headers)
    } else {
        None
    }
    .or_else(|| peer.map(|peer| peer.ip().to_string()));
    service::ExportActor {
        actor_id: session.user.id,
        actor_email: session.user.email.clone(),
        client_ip,
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string(),
        request_id: request_id_from_headers(headers),
    }
}

/// `X-Forwarded-For` 最左合法 IP（仅在 `GW_TRUST_PROXY_HEADERS=true` 时采信）。
fn forwarded_client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_string())
}

fn csv_response(export: service::CsvExport) -> Result<Response, AppError> {
    let disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", export.filename))
            .map_err(|error| AppError::internal(format!("invalid csv filename: {error}")))?;
    let mut response = Response::new(Body::from(export.body));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    headers.insert(header::CONTENT_DISPOSITION, disposition);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
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
