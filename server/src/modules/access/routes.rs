//! access HTTP handlers：管理台 users/pairing-codes/devices/keys + 客户端 pair/token/device。
//!
//! 契约：`contracts/access.yaml` v0.2.0。

use std::{collections::BTreeMap, net::SocketAddr};

use axum::{
    Json, Router,
    extract::{
        ConnectInfo, DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::platform::{error::AppError, state::AppState};
use crate::{
    modules::identity::middleware::AdminSession, modules::identity::middleware::require_csrf,
};

use super::{
    model::{
        ClientPairRequest, CreatePairingCodeRequest, CreateUserGroupRequest, MAX_BODY_BYTES,
        PageQuery, PatchDeviceRequest, PatchKeyRequest, PatchUserRequest, TokenRequest,
        UpdateUserGroupRequest, UserGroupQuery,
    },
    service,
};

pub fn client_router() -> Router<AppState> {
    Router::new()
        .route("/api/client/pair", post(pair))
        .route("/api/client/token", post(exchange_token))
        .route("/api/client/device", get(client_device))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(list_users))
        .route("/api/admin/users/{user_id}", patch(update_user))
        .route(
            "/api/admin/pairing-codes",
            get(list_pairing_codes).post(create_pairing_code),
        )
        .route(
            "/api/admin/pairing-codes/{pairing_code_id}",
            delete(invalidate_pairing_code),
        )
        .route("/api/admin/devices", get(list_devices))
        .route(
            "/api/admin/devices/{device_id}",
            get(get_device).patch(update_device).delete(unbind_device),
        )
        .route("/api/admin/keys", get(list_keys))
        .route(
            "/api/admin/keys/{key_id}",
            get(get_key).patch(update_key).delete(revoke_key),
        )
        .route("/api/admin/keys/{key_id}/reset", post(reset_key))
        .route(
            "/api/admin/user-groups",
            get(list_user_groups).post(create_user_group),
        )
        .route(
            "/api/admin/user-groups/{user_group_id}",
            get(get_user_group)
                .patch(update_user_group)
                .delete(delete_user_group),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

// ---------- 管理台 ----------

async fn list_users(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_users(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn update_user(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    payload: Result<Json<PatchUserRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let user_id = parse_uuid(&user_id, "user_not_found", "员工用户不存在")?;
    let request = json_body(payload)?;
    let user = service::update_user(&state, user_id, request).await?;
    Ok(no_store_json(user))
}

async fn create_pairing_code(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<CreatePairingCodeRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let created = service::create_pairing_code(&state, session.user.id, request).await?;
    let mut response = (StatusCode::CREATED, Json(created)).into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn list_pairing_codes(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_pairing_codes(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn invalidate_pairing_code(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(pairing_code_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let pairing_code_id = parse_uuid(
        &pairing_code_id,
        "pairing_code_not_found",
        "配对码记录不存在",
    )?;
    service::invalidate_pairing_code(&state, pairing_code_id).await?;
    Ok(no_content())
}

async fn list_devices(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_devices(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn get_device(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(device_id): Path<String>,
) -> Result<Response, AppError> {
    let device_id = parse_uuid(&device_id, "device_not_found", "设备不存在")?;
    let detail = service::get_device(&state, device_id).await?;
    Ok(no_store_json(detail))
}

async fn update_device(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    payload: Result<Json<PatchDeviceRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let device_id = parse_uuid(&device_id, "device_not_found", "设备不存在")?;
    let request = json_body(payload)?;
    let device = service::update_device(&state, device_id, request).await?;
    Ok(no_store_json(device))
}

async fn unbind_device(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let device_id = parse_uuid(&device_id, "device_not_found", "设备不存在")?;
    service::unbind_device(&state, device_id).await?;
    Ok(no_content())
}

async fn list_keys(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_keys(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn get_key(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(key_id): Path<String>,
) -> Result<Response, AppError> {
    let key_id = parse_uuid(&key_id, "key_not_found", "虚拟 Key 不存在")?;
    let key = service::get_key(&state, key_id).await?;
    Ok(no_store_json(key))
}

async fn update_key(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    payload: Result<Json<PatchKeyRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let key_id = parse_uuid(&key_id, "key_not_found", "虚拟 Key 不存在")?;
    let request = json_body(payload)?;
    let key = service::update_key(&state, key_id, request).await?;
    Ok(no_store_json(key))
}

async fn revoke_key(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let key_id = parse_uuid(&key_id, "key_not_found", "虚拟 Key 不存在")?;
    service::revoke_key(&state, key_id).await?;
    Ok(no_content())
}

async fn reset_key(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let key_id = parse_uuid(&key_id, "key_not_found", "虚拟 Key 不存在")?;
    let reset = service::reset_key(&state, key_id).await?;
    Ok(no_store_json(reset))
}

async fn list_user_groups(
    State(state): State<AppState>,
    _session: AdminSession,
    query: Result<Query<UserGroupQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let query = parse_query(query)?;
    let page = service::list_user_groups(&state, &query).await?;
    Ok(no_store_json(page))
}

async fn create_user_group(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<CreateUserGroupRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = json_body(payload)?;
    let group = service::create_user_group(&state, request).await?;
    let mut response = (StatusCode::CREATED, Json(group)).into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn get_user_group(
    State(state): State<AppState>,
    _session: AdminSession,
    Path(user_group_id): Path<String>,
) -> Result<Response, AppError> {
    let user_group_id = parse_uuid(&user_group_id, "user_group_not_found", "用户组不存在")?;
    let group = service::get_user_group_detail(&state, user_group_id).await?;
    Ok(no_store_json(group))
}

async fn update_user_group(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(user_group_id): Path<String>,
    payload: Result<Json<UpdateUserGroupRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let user_group_id = parse_uuid(&user_group_id, "user_group_not_found", "用户组不存在")?;
    let request = json_body(payload)?;
    let group = service::update_user_group(&state, user_group_id, request).await?;
    Ok(no_store_json(group))
}

async fn delete_user_group(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    Path(user_group_id): Path<String>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let user_group_id = parse_uuid(&user_group_id, "user_group_not_found", "用户组不存在")?;
    service::delete_user_group(&state, user_group_id).await?;
    Ok(no_content())
}

// ---------- 客户端 ----------

async fn pair(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    payload: Result<Json<ClientPairRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let result = service::pair_client(&state, request, &peer.ip().to_string()).await?;
    let mut response = (StatusCode::CREATED, Json(result)).into_response();
    add_no_store(&mut response);
    Ok(response)
}

async fn exchange_token(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    payload: Result<Json<TokenRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = json_body(payload)?;
    let result = service::exchange_token(&state, request, &peer.ip().to_string()).await?;
    Ok(no_store_json(result))
}

async fn client_device(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let token = bearer_token(&headers)?;
    let result = service::client_device(&state, token, &peer.ip().to_string()).await?;
    Ok(no_store_json(result))
}

// ---------- helpers ----------

fn bearer_token(headers: &HeaderMap) -> Result<&str, AppError> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim();
    let Some(token) = raw.strip_prefix("Bearer ") else {
        return Err(AppError::Unauthorized {
            code: "invalid_device_token",
            detail: "设备 token 无效或已过期",
        });
    };
    let token = token.trim();
    if token.is_empty() {
        return Err(AppError::Unauthorized {
            code: "invalid_device_token",
            detail: "设备 token 无效或已过期",
        });
    }
    Ok(token)
}

fn parse_uuid(raw: &str, code: &'static str, detail: &'static str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found(code, detail))
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
