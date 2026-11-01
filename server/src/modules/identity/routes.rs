//! identity HTTP handlers 与路由装配。契约见 `接口设计.md` §3–§6。

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::modules::observability::{
    model::AuditEvent,
    service::{app_error_code, record_audit_detached},
};
use crate::platform::{
    error::AppError, http as platform_http, http::request_id_from_headers, state::AppState,
};

use super::{
    middleware::{AdminSession, require_csrf},
    model::{
        AdminUser, ChangePasswordRequest, LoginRequest, LoginResponse, SESSION_COOKIE,
        SESSION_TTL_SECS, SetupRequest, SetupStatus,
    },
    service,
};

const MAX_BODY_BYTES: usize = 64 * 1024;
const CSRF_HEADER: &str = "x-csrf-token";
const SETUP_TOKEN_HEADER: &str = "x-setup-token";

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/auth/setup",
            get(get_setup_status).post(post_setup),
        )
        .route("/api/admin/auth/login", post(login))
        .route("/api/admin/auth/me", get(me))
        .route("/api/admin/auth/logout", post(logout))
        .route("/api/admin/auth/password", patch(change_password))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

async fn get_setup_status(State(state): State<AppState>) -> Result<Response, AppError> {
    let status: SetupStatus = service::setup_status(&state).await?;
    Ok(no_store_json(status))
}

async fn post_setup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    payload: Result<Json<SetupRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    // 显式审计失败也要记录尝试邮箱（脱敏写入，绝不记录密码）。
    let attempted_email = request.email.clone();
    let setup_token = header_string(&headers, SETUP_TOKEN_HEADER);
    let peer_ip = peer.ip();
    let user: AdminUser = match service::setup(
        &state,
        request,
        setup_token.as_deref(),
        &peer_ip.to_string(),
    )
    .await
    {
        Ok(user) => {
            record_audit_detached(
                &state,
                admin_auth_audit_event(
                    &headers,
                    peer_ip,
                    "identity.auth.setup",
                    &user,
                    serde_json::json!({}),
                ),
            );
            user
        }
        Err(error) => {
            record_audit_detached(
                &state,
                anonymous_auth_audit_event(
                    &headers,
                    peer_ip,
                    "identity.auth.setup",
                    Some(attempted_email),
                    serde_json::json!({ "error_code": app_error_code(&error) }),
                ),
            );
            return Err(error);
        }
    };
    let mut response = (StatusCode::CREATED, Json(user)).into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let attempted_email = request.email.clone();
    let user_agent = header_string(&headers, header::USER_AGENT.as_str()).unwrap_or_default();
    let peer_ip = peer.ip();
    let (body, session_token) = match service::login(
        &state,
        request,
        &peer_ip.to_string(),
        &user_agent.chars().take(512).collect::<String>(),
    )
    .await
    {
        Ok(result) => {
            record_audit_detached(
                &state,
                admin_auth_audit_event(
                    &headers,
                    peer_ip,
                    "identity.auth.login",
                    &result.0.user,
                    serde_json::json!({}),
                ),
            );
            result
        }
        Err(error) => {
            record_audit_detached(
                &state,
                anonymous_auth_audit_event(
                    &headers,
                    peer_ip,
                    "identity.auth.login",
                    Some(attempted_email),
                    serde_json::json!({ "error_code": app_error_code(&error) }),
                ),
            );
            return Err(error);
        }
    };

    let mut response = Json(body).into_response();
    add_no_store(&mut response);
    add_set_cookie(
        &mut response,
        platform_http::session_cookie(
            SESSION_COOKIE,
            &session_token,
            SESSION_TTL_SECS,
            state.config.cookie_secure,
        ),
    );
    Ok(response)
}

async fn me(session: AdminSession) -> Result<Response, AppError> {
    let body = LoginResponse {
        user: session.user,
        csrf_token: session.csrf_token,
        expires_at: session.expires_at,
    };
    Ok(no_store_json(body))
}

async fn change_password(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<ChangePasswordRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    service::change_password(&state, &session, request).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn logout(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    service::logout(&state, &session.token_hash).await?;

    let mut response = StatusCode::NO_CONTENT.into_response();
    add_no_store(&mut response);
    add_set_cookie(
        &mut response,
        platform_http::clear_cookie(SESSION_COOKIE, state.config.cookie_secure),
    );
    Ok(response)
}

fn no_store_json<T: Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    add_no_store(&mut response);
    response
}

fn add_no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

fn add_set_cookie(response: &mut Response, cookie: String) {
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
}

/// 审计公共上下文：client_ip（identity 按 peer IP）、UA、platform request_id。
fn auth_audit_context(
    headers: &HeaderMap,
    peer_ip: std::net::IpAddr,
) -> (Option<String>, String, Option<String>) {
    (
        Some(peer_ip.to_string()),
        headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string(),
        request_id_from_headers(headers),
    )
}

/// 管理员显式审计事件（登录/setup 成功）。
fn admin_auth_audit_event(
    headers: &HeaderMap,
    peer_ip: std::net::IpAddr,
    action: &str,
    user: &AdminUser,
    detail: serde_json::Value,
) -> AuditEvent {
    let (client_ip, user_agent, request_id) = auth_audit_context(headers, peer_ip);
    AuditEvent {
        actor_type: "admin".to_string(),
        actor_id: Some(user.id),
        actor_email: Some(user.email.clone()),
        action: action.to_string(),
        resource_type: "admin_user".to_string(),
        resource_id: Some(user.id.to_string()),
        outcome: "success".to_string(),
        client_ip,
        user_agent,
        request_id,
        detail,
    }
}

/// 匿名显式审计事件（登录/setup 失败；仅尝试邮箱，不含密码）。
fn anonymous_auth_audit_event(
    headers: &HeaderMap,
    peer_ip: std::net::IpAddr,
    action: &str,
    attempted_email: Option<String>,
    detail: serde_json::Value,
) -> AuditEvent {
    let (client_ip, user_agent, request_id) = auth_audit_context(headers, peer_ip);
    AuditEvent {
        actor_type: "anonymous".to_string(),
        actor_id: None,
        actor_email: attempted_email,
        action: action.to_string(),
        resource_type: "admin_user".to_string(),
        resource_id: None,
        outcome: "failure".to_string(),
        client_ip,
        user_agent,
        request_id,
        detail,
    }
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

// CSRF 头名常量由 middleware 使用；保留此处引用避免未来漏改。
#[allow(dead_code)]
fn csrf_header_name() -> &'static str {
    CSRF_HEADER
}
