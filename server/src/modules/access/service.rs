//! access 用例：管理台用户/配对码/设备/Key + 客户端配对/token/device + S4 内部鉴权。
//!
//! 行为以 `设计文档（暂未开源）` V0.2 为准。

use std::collections::HashSet;
use std::time::Instant;

use axum::http::{HeaderMap, header};
use chrono::{Duration, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{
        AuthContext, ClientDeviceInfo, ClientDeviceResponse, ClientDeviceSummary, ClientKeySummary,
        ClientPairRequest, ClientPairResponse, CreatePairingCodeRequest, CreatePairingCodeResponse,
        CreateUserGroupRequest, DEFAULT_PAGE_LIMIT, DEVICE_ONLINE_WINDOW_SECS,
        DEVICE_TOKEN_TTL_SECS, DeviceAdmin, DeviceDetail, DeviceInput, DeviceKeySummary, KeyAdmin,
        KeyDevice, MAX_ALLOWED_MODELS, MAX_CLIENT_VERSION_CHARS, MAX_DEVICE_NAME_CHARS,
        MAX_DISPLAY_NAME_CHARS, MAX_MODEL_NAME_CHARS, MAX_PAGE_LIMIT, MAX_PLATFORM_CHARS,
        MAX_USER_GROUP_DESCRIPTION_CHARS, MAX_USER_GROUP_MEMBERS, MAX_USER_GROUP_NAME_CHARS,
        PAIRING_CODE_MAX_TTL_SECS, PAIRING_CODE_MIN_TTL_SECS, PAIRING_CODE_TTL_SECS, Page,
        PageQuery, PairingCode, PairingCodeWithCode, PatchDeviceRequest, PatchKeyRequest,
        PatchUserRequest, ResetKeyResponse, TokenRequest, TokenResponse, UpdateUserGroupRequest,
        User, UserGroupDetail, UserGroupQuery, UserGroupSummary,
    },
    rate_limit, repo,
    security::{self, validate_optional_text},
};

// ---------- helpers ----------

pub fn db(state: &AppState) -> Result<&sqlx::PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
}

/// 分页参数校验（列表端点共用）：limit 1–MAX_PAGE_LIMIT、offset ≥0。
fn page_limits(limit: Option<i64>, offset: Option<i64>) -> Result<(i64, i64), AppError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err(AppError::validation_field(
            "limit",
            format!("limit 必须在 1–{MAX_PAGE_LIMIT}"),
        ));
    }
    let offset = offset.unwrap_or(0);
    if offset < 0 {
        return Err(AppError::validation_field("offset", "offset 不能为负数"));
    }
    Ok((limit, offset))
}

fn page_params(query: &PageQuery) -> Result<(i64, i64), AppError> {
    page_limits(query.limit, query.offset)
}

fn parse_uuid_filter(raw: Option<&str>, field: &str) -> Result<Option<Uuid>, AppError> {
    match raw {
        None => Ok(None),
        Some(value) => Uuid::parse_str(value.trim())
            .map(Some)
            .map_err(|_| AppError::validation_field(field, format!("{field} 不是合法 UUID"))),
    }
}

fn validate_status_filter(
    raw: Option<&str>,
    allowed: &[&str],
    field: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = raw else { return Ok(None) };
    let value = value.trim();
    if !allowed.contains(&value) {
        return Err(AppError::validation_field(
            field,
            format!("{field} 必须是 {}", allowed.join("/")),
        ));
    }
    Ok(Some(value.to_string()))
}

fn validate_patch_text(
    raw: Option<&str>,
    field: &str,
    max: usize,
) -> Result<Option<String>, AppError> {
    match raw {
        None => Ok(None),
        Some(value) => validate_optional_text(Some(value), field, max)
            .map(Some)
            .map_err(|message| AppError::validation_field(field, message)),
    }
}

fn validate_allowed_models(models: &[String]) -> Result<Vec<String>, AppError> {
    if models.len() > MAX_ALLOWED_MODELS {
        return Err(AppError::validation_field(
            "allowed_models",
            format!("allowed_models 最多 {MAX_ALLOWED_MODELS} 项"),
        ));
    }
    let mut out = Vec::with_capacity(models.len());
    for model in models {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return Err(AppError::validation_field(
                "allowed_models",
                "模型名不能为空",
            ));
        }
        if trimmed.chars().count() > MAX_MODEL_NAME_CHARS {
            return Err(AppError::validation_field(
                "allowed_models",
                format!("模型名不能超过 {MAX_MODEL_NAME_CHARS} 个字符"),
            ));
        }
        out.push(trimmed.to_string());
    }
    Ok(out)
}

fn to_user(row: &repo::UserRow) -> User {
    User {
        id: row.id,
        email: row.email.clone(),
        display_name: row.display_name.clone(),
        status: row.status.clone(),
    }
}

fn pairing_status(row: &repo::PairingCodeRow) -> String {
    if row.used_at.is_some() {
        "used"
    } else if row.invalidated_at.is_some() {
        "invalidated"
    } else if row.expires_at <= Utc::now() {
        "expired"
    } else {
        "active"
    }
    .to_string()
}

fn to_pairing_code(row: &repo::PairingCodeRow) -> PairingCode {
    PairingCode {
        id: row.id,
        user: User {
            id: row.user_id,
            email: row.user_email.clone(),
            display_name: row.user_display_name.clone(),
            status: row.user_status.clone(),
        },
        status: pairing_status(row),
        expires_at: row.expires_at,
        used_at: row.used_at,
        invalidated_at: row.invalidated_at,
        created_at: row.created_at,
    }
}

fn to_device_admin(row: &repo::DeviceAdminRow) -> DeviceAdmin {
    let online = row.status == "active"
        && row.last_seen_at.is_some_and(|seen| {
            Utc::now().signed_duration_since(seen).num_seconds() < DEVICE_ONLINE_WINDOW_SECS
        });
    DeviceAdmin {
        id: row.id,
        user: User {
            id: row.user_id,
            email: row.user_email.clone(),
            display_name: row.user_display_name.clone(),
            status: row.user_status.clone(),
        },
        name: row.name.clone(),
        platform: row.platform.clone(),
        client_version: row.client_version.clone(),
        status: row.status.clone(),
        online,
        last_seen_at: row.last_seen_at,
        last_ip: row.last_ip.clone(),
        key: row.key_id.map(|id| DeviceKeySummary {
            id,
            key_prefix: row.key_prefix.clone().unwrap_or_default(),
            status: row.key_status.clone().unwrap_or_default(),
            expires_at: row.key_expires_at,
        }),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn to_key_admin(row: &repo::KeyAdminRow) -> KeyAdmin {
    KeyAdmin {
        id: row.id,
        key_prefix: row.key_prefix.clone(),
        status: row.status.clone(),
        user: User {
            id: row.user_id,
            email: row.user_email.clone(),
            display_name: row.user_display_name.clone(),
            status: row.user_status.clone(),
        },
        device: KeyDevice {
            id: row.device_id,
            name: row.device_name.clone(),
        },
        allowed_models: row.allowed_models.clone(),
        rpm_limit: row.rpm_limit,
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        revoked_at: row.revoked_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn pairing_invalid() -> AppError {
    AppError::Unauthorized {
        code: "pairing_code_invalid",
        detail: "配对码无效、已过期或已被使用",
    }
}

fn invalid_device_credentials() -> AppError {
    AppError::Unauthorized {
        code: "invalid_device_credentials",
        detail: "设备凭证无效",
    }
}

fn invalid_device_token() -> AppError {
    AppError::Unauthorized {
        code: "invalid_device_token",
        detail: "设备 token 无效或已过期",
    }
}

fn user_disabled() -> AppError {
    AppError::Forbidden {
        code: "user_disabled",
        detail: "该用户已被禁用".to_string(),
    }
}

fn device_disabled() -> AppError {
    AppError::Forbidden {
        code: "device_disabled",
        detail: "该设备已被禁用或解绑".to_string(),
    }
}

fn user_not_found() -> AppError {
    AppError::not_found("user_not_found", "员工用户不存在")
}

fn device_not_found() -> AppError {
    AppError::not_found("device_not_found", "设备不存在")
}

fn key_not_found() -> AppError {
    AppError::not_found("key_not_found", "虚拟 Key 不存在")
}

fn validate_device_input(
    input: Option<&DeviceInput>,
) -> Result<(String, String, String), AppError> {
    let name = validate_patch_text(
        input.and_then(|value| value.name.as_deref()),
        "device.name",
        MAX_DEVICE_NAME_CHARS,
    )?
    .unwrap_or_default();
    let platform = validate_patch_text(
        input.and_then(|value| value.platform.as_deref()),
        "device.platform",
        MAX_PLATFORM_CHARS,
    )?
    .unwrap_or_default();
    let client_version = validate_patch_text(
        input.and_then(|value| value.client_version.as_deref()),
        "device.client_version",
        MAX_CLIENT_VERSION_CHARS,
    )?
    .unwrap_or_default();
    Ok((name, platform, client_version))
}

fn to_client_device(row: &repo::TokenValidateRow) -> ClientDeviceInfo {
    ClientDeviceInfo {
        id: row.device_id,
        name: row.device_name.clone(),
        platform: row.device_platform.clone(),
        client_version: row.device_client_version.clone(),
        status: row.device_status.clone(),
        last_seen_at: row.device_last_seen_at,
    }
}

// ---------- 管理台：users ----------

/// 内部只读：按 id 取员工（catalog 发布对象/预览校验用；不暴露 HTTP）。
pub async fn get_user_by_id(state: &AppState, user_id: Uuid) -> Result<Option<User>, AppError> {
    let pool = db(state)?;
    Ok(repo::get_user(pool, user_id).await?.as_ref().map(to_user))
}

pub async fn list_users(state: &AppState, query: &PageQuery) -> Result<Page<User>, AppError> {
    let pool = db(state)?;
    let status =
        validate_status_filter(query.status.as_deref(), &["active", "disabled"], "status")?;
    let (limit, offset) = page_params(query)?;
    let (rows, total) = repo::list_users(pool, status.as_deref(), limit, offset).await?;
    Ok(Page {
        items: rows.iter().map(to_user).collect(),
        total,
        limit,
        offset,
    })
}

pub async fn update_user(
    state: &AppState,
    id: Uuid,
    request: PatchUserRequest,
) -> Result<User, AppError> {
    let pool = db(state)?;
    let current = repo::get_user(pool, id).await?.ok_or_else(user_not_found)?;
    if request.display_name.is_none() && request.status.is_none() {
        return Err(AppError::validation_field("body", "至少提供一个字段"));
    }
    let display_name = match request.display_name {
        Some(raw) => security::validate_user_display_name(&raw)
            .map_err(|message| AppError::validation_field("display_name", message))?,
        None => current.display_name.clone(),
    };
    let status = match request.status {
        Some(raw) => validate_status_filter(Some(&raw), &["active", "disabled"], "status")?
            .expect("status provided"),
        None => current.status.clone(),
    };
    if display_name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(AppError::validation_field(
            "display_name",
            format!("显示名不能超过 {MAX_DISPLAY_NAME_CHARS} 个字符"),
        ));
    }
    let mut tx = pool.begin().await?;
    let updated = repo::update_user_tx(&mut tx, id, &display_name, &status)
        .await?
        .ok_or_else(user_not_found)?;
    if status == "disabled" {
        repo::delete_user_tokens_tx(&mut tx, id).await?;
    }
    tx.commit().await?;
    Ok(to_user(&updated))
}

// ---------- 管理台：pairing codes ----------

pub async fn create_pairing_code(
    state: &AppState,
    admin_user_id: Uuid,
    request: CreatePairingCodeRequest,
) -> Result<CreatePairingCodeResponse, AppError> {
    let pool = db(state)?;
    let email = security::validate_email_address(&request.user_email)
        .map_err(|message| AppError::validation_field("user_email", message))?;
    let display_name = match request.user_display_name.as_deref() {
        Some(raw) => security::validate_user_display_name(raw)
            .map_err(|message| AppError::validation_field("user_display_name", message))?,
        None => String::new(),
    };
    let ttl = request.expires_in_seconds.unwrap_or(PAIRING_CODE_TTL_SECS);
    if !(PAIRING_CODE_MIN_TTL_SECS..=PAIRING_CODE_MAX_TTL_SECS).contains(&ttl) {
        return Err(AppError::validation_field(
            "expires_in_seconds",
            format!(
                "expires_in_seconds 必须在 {PAIRING_CODE_MIN_TTL_SECS}–{PAIRING_CODE_MAX_TTL_SECS}"
            ),
        ));
    }

    let mut tx = pool.begin().await?;
    let user = repo::find_or_create_user(&mut tx, &email, &display_name).await?;
    if user.status != "active" {
        return Err(user_disabled());
    }
    repo::invalidate_active_codes_tx(&mut tx, user.id).await?;

    let plain_code = security::generate_pairing_code();
    let normalized = security::normalize_pairing_code(&plain_code)
        .map_err(|_| AppError::internal("generated pairing code is invalid"))?;
    let code_hash = security::hash_pairing_code(&normalized);
    let id = Uuid::new_v4();
    let created_at = Utc::now();
    let expires_at = created_at + Duration::seconds(ttl);
    repo::insert_pairing_code(&mut tx, id, &code_hash, user.id, admin_user_id, expires_at).await?;
    tx.commit().await?;

    Ok(CreatePairingCodeResponse {
        pairing_code: PairingCodeWithCode {
            meta: PairingCode {
                id,
                user: to_user(&user),
                status: "active".to_string(),
                expires_at,
                used_at: None,
                invalidated_at: None,
                created_at,
            },
            code: plain_code,
        },
    })
}

pub async fn list_pairing_codes(
    state: &AppState,
    query: &PageQuery,
) -> Result<Page<PairingCode>, AppError> {
    let pool = db(state)?;
    let status = validate_status_filter(
        query.status.as_deref(),
        &["active", "used", "expired", "invalidated"],
        "status",
    )?;
    let user_id = parse_uuid_filter(query.user_id.as_deref(), "user_id")?;
    let (limit, offset) = page_params(query)?;
    let (rows, total) =
        repo::list_pairing_codes(pool, status.as_deref(), user_id, limit, offset).await?;
    Ok(Page {
        items: rows.iter().map(to_pairing_code).collect(),
        total,
        limit,
        offset,
    })
}

pub async fn invalidate_pairing_code(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    if !repo::pairing_code_exists(pool, id).await? {
        return Err(AppError::not_found(
            "pairing_code_not_found",
            "配对码记录不存在",
        ));
    }
    repo::invalidate_pairing_code(pool, id).await?;
    Ok(())
}

// ---------- 管理台：devices ----------

pub async fn list_devices(
    state: &AppState,
    query: &PageQuery,
) -> Result<Page<DeviceAdmin>, AppError> {
    let pool = db(state)?;
    let status =
        validate_status_filter(query.status.as_deref(), &["active", "disabled"], "status")?;
    let user_id = parse_uuid_filter(query.user_id.as_deref(), "user_id")?;
    let (limit, offset) = page_params(query)?;
    let (rows, total) = repo::list_devices(pool, status.as_deref(), user_id, limit, offset).await?;
    Ok(Page {
        items: rows.iter().map(to_device_admin).collect(),
        total,
        limit,
        offset,
    })
}

pub async fn get_device(state: &AppState, id: Uuid) -> Result<DeviceDetail, AppError> {
    let pool = db(state)?;
    let row = repo::get_device_admin(pool, id)
        .await?
        .ok_or_else(device_not_found)?;
    let (active_token_count, last_token_expires_at) = repo::device_token_stats(pool, id).await?;
    Ok(DeviceDetail {
        device: to_device_admin(&row),
        active_token_count,
        last_token_expires_at,
    })
}

pub async fn update_device(
    state: &AppState,
    id: Uuid,
    request: PatchDeviceRequest,
) -> Result<DeviceAdmin, AppError> {
    let pool = db(state)?;
    let current = repo::get_device_admin(pool, id)
        .await?
        .ok_or_else(device_not_found)?;
    if request.name.is_none() && request.status.is_none() {
        return Err(AppError::validation_field("body", "至少提供一个字段"));
    }
    let name = match request.name {
        Some(raw) => validate_optional_text(Some(&raw), "name", MAX_DEVICE_NAME_CHARS)
            .map_err(|message| AppError::validation_field("name", message))?,
        None => current.name.clone(),
    };
    let status = match request.status {
        Some(raw) => validate_status_filter(Some(&raw), &["active", "disabled"], "status")?
            .expect("status provided"),
        None => current.status.clone(),
    };

    let mut tx = pool.begin().await?;
    repo::update_device_tx(&mut tx, id, &name, &status).await?;
    if status == "disabled" {
        repo::delete_device_tokens_tx(&mut tx, id).await?;
        repo::set_key_status(&mut tx, id, "paused").await?;
    } else {
        repo::set_key_status(&mut tx, id, "active").await?;
    }
    tx.commit().await?;

    let row = repo::get_device_admin(pool, id)
        .await?
        .ok_or_else(device_not_found)?;
    Ok(to_device_admin(&row))
}

pub async fn unbind_device(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    if repo::get_device_admin(pool, id).await?.is_none() {
        return Err(device_not_found());
    }
    let mut tx = pool.begin().await?;
    repo::set_device_status(&mut tx, id, "disabled").await?;
    repo::delete_device_tokens_tx(&mut tx, id).await?;
    repo::set_key_status(&mut tx, id, "revoked").await?;
    tx.commit().await?;
    Ok(())
}

// ---------- 管理台：virtual keys ----------

pub async fn list_keys(state: &AppState, query: &PageQuery) -> Result<Page<KeyAdmin>, AppError> {
    let pool = db(state)?;
    let status = validate_status_filter(
        query.status.as_deref(),
        &["active", "paused", "revoked"],
        "status",
    )?;
    let user_id = parse_uuid_filter(query.user_id.as_deref(), "user_id")?;
    let device_id = parse_uuid_filter(query.device_id.as_deref(), "device_id")?;
    let (limit, offset) = page_params(query)?;
    let (rows, total) =
        repo::list_keys(pool, status.as_deref(), user_id, device_id, limit, offset).await?;
    Ok(Page {
        items: rows.iter().map(to_key_admin).collect(),
        total,
        limit,
        offset,
    })
}

pub async fn get_key(state: &AppState, id: Uuid) -> Result<KeyAdmin, AppError> {
    let pool = db(state)?;
    let row = repo::get_key_admin(pool, id)
        .await?
        .ok_or_else(key_not_found)?;
    Ok(to_key_admin(&row))
}

pub async fn update_key(
    state: &AppState,
    id: Uuid,
    request: PatchKeyRequest,
) -> Result<KeyAdmin, AppError> {
    let pool = db(state)?;
    let current = repo::get_key_admin(pool, id)
        .await?
        .ok_or_else(key_not_found)?;
    if request.allowed_models.is_none()
        && request.expires_at.is_none()
        && request.rpm_limit.is_none()
    {
        return Err(AppError::validation_field("body", "至少提供一个字段"));
    }
    let allowed_models = match request.allowed_models {
        Some(models) => validate_allowed_models(&models)?,
        None => current.allowed_models.clone(),
    };
    let expires_at = match request.expires_at {
        None => current.expires_at,
        Some(None) => None,
        Some(Some(value)) => {
            if value <= Utc::now() {
                return Err(AppError::validation_field(
                    "expires_at",
                    "expires_at 必须晚于当前时间",
                ));
            }
            Some(value)
        }
    };
    let rpm_limit = match request.rpm_limit {
        None => current.rpm_limit,
        Some(None) => None,
        Some(Some(value)) => {
            if !(1..=1_000_000).contains(&value) {
                return Err(AppError::validation_field(
                    "rpm_limit",
                    "rpm_limit 必须在 1–1,000,000 之间",
                ));
            }
            Some(value)
        }
    };
    repo::update_key(pool, id, &allowed_models, expires_at, rpm_limit).await?;
    let row = repo::get_key_admin(pool, id)
        .await?
        .ok_or_else(key_not_found)?;
    Ok(to_key_admin(&row))
}

pub async fn revoke_key(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    if !repo::revoke_key(pool, id).await? {
        return Err(key_not_found());
    }
    Ok(())
}

pub async fn reset_key(state: &AppState, id: Uuid) -> Result<ResetKeyResponse, AppError> {
    let pool = db(state)?;
    if repo::get_key_admin(pool, id).await?.is_none() {
        return Err(key_not_found());
    }
    let device_id = repo::get_key_admin(pool, id)
        .await?
        .ok_or_else(key_not_found)?
        .device_id;
    let (api_key, prefix) = security::generate_api_key();
    let key_hash = security::sha256_hex(&api_key);
    let mut tx = pool.begin().await?;
    if !repo::reset_key(&mut tx, id, &prefix, &key_hash).await? {
        return Err(key_not_found());
    }
    repo::delete_device_tokens_tx(&mut tx, device_id).await?;
    tx.commit().await?;
    let row = repo::get_key_admin(pool, id)
        .await?
        .ok_or_else(key_not_found)?;
    Ok(ResetKeyResponse {
        api_key,
        key: to_key_admin(&row),
    })
}

// ---------- 管理台：user groups（US-041 发布对象依赖） ----------

fn user_group_not_found() -> AppError {
    AppError::not_found("user_group_not_found", "用户组不存在")
}

/// trim 后 1–64 字符（与 0013 CHECK 一致）。
fn validate_user_group_name(raw: &str) -> Result<String, AppError> {
    let name = validate_optional_text(Some(raw), "name", MAX_USER_GROUP_NAME_CHARS)
        .map_err(|message| AppError::validation_field("name", message))?;
    if name.is_empty() {
        return Err(AppError::validation_field("name", "用户组名称不能为空"));
    }
    Ok(name)
}

/// trim 后 ≤200 字符；缺省=空串。
fn validate_user_group_description(raw: Option<&str>) -> Result<String, AppError> {
    validate_optional_text(raw, "description", MAX_USER_GROUP_DESCRIPTION_CHARS)
        .map_err(|message| AppError::validation_field("description", message))
}

/// 去重（保留首次出现顺序）并校验成员上限；不校验用户存在性（需要事务）。
fn normalize_member_ids(raw: &[Uuid]) -> Result<Vec<Uuid>, AppError> {
    let mut seen = HashSet::with_capacity(raw.len());
    let mut out = Vec::with_capacity(raw.len().min(MAX_USER_GROUP_MEMBERS));
    for id in raw {
        if !seen.insert(*id) {
            continue;
        }
        if out.len() == MAX_USER_GROUP_MEMBERS {
            return Err(AppError::validation_field(
                "member_ids",
                format!("member_ids 最多 {MAX_USER_GROUP_MEMBERS} 项"),
            ));
        }
        out.push(*id);
    }
    Ok(out)
}

fn user_group_query_params(query: &UserGroupQuery) -> Result<(Option<String>, i64, i64), AppError> {
    let q = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let (limit, offset) = page_limits(query.limit, query.offset)?;
    Ok((q, limit, offset))
}

/// 事务内校验成员用户存在性（disabled 允许入组，见实现评审 Medium）。
async fn ensure_member_users_exist(
    conn: &mut PgConnection,
    member_ids: &[Uuid],
) -> Result<(), AppError> {
    if member_ids.is_empty() {
        return Ok(());
    }
    let existing = repo::existing_user_ids(conn, member_ids).await?;
    if existing.len() != member_ids.len() {
        return Err(user_not_found());
    }
    Ok(())
}

fn to_user_group_summary(row: &repo::UserGroupSummaryRow) -> UserGroupSummary {
    UserGroupSummary {
        id: row.id,
        name: row.name.clone(),
        description: row.description.clone(),
        member_count: row.member_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn build_user_group_detail(
    pool: &sqlx::PgPool,
    id: Uuid,
) -> Result<UserGroupDetail, AppError> {
    let row = repo::get_user_group(pool, id)
        .await?
        .ok_or_else(user_group_not_found)?;
    let members = repo::list_user_group_members(pool, id).await?;
    Ok(UserGroupDetail {
        id: row.id,
        name: row.name,
        description: row.description,
        members: members.iter().map(to_user).collect(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn list_user_groups(
    state: &AppState,
    query: &UserGroupQuery,
) -> Result<Page<UserGroupSummary>, AppError> {
    let pool = db(state)?;
    let (q, limit, offset) = user_group_query_params(query)?;
    let (rows, total) = repo::list_user_groups(pool, q.as_deref(), limit, offset).await?;
    Ok(Page {
        items: rows.iter().map(to_user_group_summary).collect(),
        total,
        limit,
        offset,
    })
}

pub async fn create_user_group(
    state: &AppState,
    request: CreateUserGroupRequest,
) -> Result<UserGroupDetail, AppError> {
    let name = validate_user_group_name(&request.name)?;
    let description = validate_user_group_description(request.description.as_deref())?;
    let member_ids = normalize_member_ids(request.member_ids.as_deref().unwrap_or_default())?;
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    ensure_member_users_exist(&mut tx, &member_ids).await?;
    let id = Uuid::new_v4();
    repo::insert_user_group(&mut tx, id, &name, &description).await?;
    repo::replace_user_group_members(&mut tx, id, &member_ids).await?;
    tx.commit().await?;
    build_user_group_detail(pool, id).await
}

pub async fn get_user_group_detail(
    state: &AppState,
    id: Uuid,
) -> Result<UserGroupDetail, AppError> {
    let pool = db(state)?;
    build_user_group_detail(pool, id).await
}

pub async fn update_user_group(
    state: &AppState,
    id: Uuid,
    request: UpdateUserGroupRequest,
) -> Result<UserGroupDetail, AppError> {
    if request.is_empty() {
        return Err(AppError::validation_field("body", "至少提供一个字段"));
    }
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    // 并发成员全量替换以组行锁串行化。
    let Some(current) = repo::get_user_group_for_update(&mut tx, id).await? else {
        tx.rollback().await.ok();
        return Err(user_group_not_found());
    };
    let name = match request.name.as_deref() {
        Some(raw) => validate_user_group_name(raw)?,
        None => current.name.clone(),
    };
    let description = match &request.description {
        None => current.description.clone(),
        Some(None) => String::new(),
        Some(Some(raw)) => validate_user_group_description(Some(raw))?,
    };
    let member_ids = match request.member_ids.as_deref() {
        None => None,
        Some(raw) => Some(normalize_member_ids(raw)?),
    };
    if let Some(member_ids) = &member_ids {
        ensure_member_users_exist(&mut tx, member_ids).await?;
    }
    repo::update_user_group(&mut tx, id, &name, &description)
        .await?
        .ok_or_else(user_group_not_found)?;
    if let Some(member_ids) = &member_ids {
        repo::replace_user_group_members(&mut tx, id, member_ids).await?;
    }
    tx.commit().await?;
    build_user_group_detail(pool, id).await
}

/// 物理删除组并级联成员；不存在也返回成功（DELETE 幂等）。
pub async fn delete_user_group(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    repo::delete_user_group(pool, id).await?;
    Ok(())
}

/// 内部只读：用户所属用户组 id（catalog 发布对象/预览校验用；不暴露 HTTP）。
#[allow(dead_code)]
pub async fn user_group_ids(state: &AppState, user_id: Uuid) -> Result<Vec<Uuid>, AppError> {
    let pool = db(state)?;
    repo::user_group_ids(pool, user_id).await
}

/// 内部只读：用户组是否存在（catalog 发布对象校验用；不暴露 HTTP）。
#[allow(dead_code)]
pub async fn user_group_exists(state: &AppState, group_id: Uuid) -> Result<bool, AppError> {
    let pool = db(state)?;
    repo::user_group_exists(pool, group_id).await
}

// ---------- 客户端：pair / token / device ----------

pub async fn pair_client(
    state: &AppState,
    request: ClientPairRequest,
    ip: &str,
) -> Result<ClientPairResponse, AppError> {
    let pool = db(state)?;
    let now = Instant::now();
    if let Some(retry) = rate_limit::global().check_pair(ip, now) {
        return Err(AppError::too_many_attempts(retry));
    }
    let normalized = match security::normalize_pairing_code(&request.code) {
        Ok(value) => value,
        Err(message) => {
            rate_limit::global().record_pair_failure(ip, now);
            return Err(AppError::validation_field("code", message));
        }
    };
    let (device_name, platform, client_version) = validate_device_input(request.device.as_ref())?;
    let code_hash = security::hash_pairing_code(&normalized);

    let mut tx = pool.begin().await?;
    let Some(code) = repo::consume_pairing_code(&mut tx, &code_hash).await? else {
        rate_limit::global().record_pair_failure(ip, now);
        return Err(pairing_invalid());
    };
    let current = Utc::now();
    if code.used_at.is_some() || code.invalidated_at.is_some() || code.expires_at <= current {
        rate_limit::global().record_pair_failure(ip, now);
        return Err(pairing_invalid());
    }
    if code.user_status != "active" {
        return Err(user_disabled());
    }

    let device_id = Uuid::new_v4();
    let secret = security::generate_device_secret();
    let secret_hash = security::sha256_hex(&secret);
    let (api_key, api_key_prefix) = security::generate_api_key();
    let key_hash = security::sha256_hex(&api_key);
    let token = security::generate_device_token();
    let token_hash = security::sha256_hex(&token);
    let token_expires_at = current + Duration::seconds(DEVICE_TOKEN_TTL_SECS);

    repo::insert_device(
        &mut tx,
        device_id,
        code.user_id,
        &device_name,
        &platform,
        &client_version,
        &secret_hash,
    )
    .await?;
    repo::insert_key(
        &mut tx,
        Uuid::new_v4(),
        device_id,
        code.user_id,
        &api_key_prefix,
        &key_hash,
    )
    .await?;
    repo::insert_device_token(
        &mut tx,
        Uuid::new_v4(),
        device_id,
        &token_hash,
        token_expires_at,
    )
    .await?;
    repo::mark_pairing_code_used(&mut tx, code.id).await?;
    tx.commit().await?;

    Ok(ClientPairResponse {
        device: ClientDeviceSummary {
            id: device_id,
            name: device_name,
            platform,
            status: "active".to_string(),
            created_at: current,
        },
        user: User {
            id: code.user_id,
            email: code.user_email,
            display_name: code.user_display_name,
            status: "active".to_string(),
        },
        device_secret: secret,
        api_key,
        api_key_prefix,
        token,
        token_type: "Bearer".to_string(),
        token_expires_at,
    })
}

pub async fn exchange_token(
    state: &AppState,
    request: TokenRequest,
    ip: &str,
) -> Result<TokenResponse, AppError> {
    let pool = db(state)?;
    let now = Instant::now();
    let device_key = request.device_id.to_string();
    if let Some(retry) = rate_limit::global().check_token(ip, &device_key, now) {
        return Err(AppError::too_many_attempts(retry));
    }
    if request.device_secret.trim().is_empty() {
        return Err(AppError::validation_field(
            "device_secret",
            "设备密钥不能为空",
        ));
    }
    let secret_hash = security::sha256_hex(&request.device_secret);
    let Some(device) = repo::find_device_for_token(pool, request.device_id, &secret_hash).await?
    else {
        rate_limit::global().record_token_failure(ip, &device_key, now);
        return Err(invalid_device_credentials());
    };
    if device.user_status != "active" {
        return Err(user_disabled());
    }
    if device.status != "active" {
        return Err(device_disabled());
    }
    let token = security::generate_device_token();
    let token_hash = security::sha256_hex(&token);
    let expires_at = Utc::now() + Duration::seconds(DEVICE_TOKEN_TTL_SECS);
    repo::delete_expired_device_tokens(pool, device.id).await?;
    let mut tx = pool.begin().await?;
    repo::insert_device_token(&mut tx, Uuid::new_v4(), device.id, &token_hash, expires_at).await?;
    tx.commit().await?;
    Ok(TokenResponse {
        token,
        token_type: "Bearer".to_string(),
        expires_at,
        device_id: device.id,
    })
}

pub async fn client_device(
    state: &AppState,
    token: &str,
    ip: &str,
) -> Result<ClientDeviceResponse, AppError> {
    let pool = db(state)?;
    let token_hash = security::sha256_hex(token);
    let Some(row) = repo::validate_device_token(pool, &token_hash).await? else {
        return Err(invalid_device_token());
    };
    if row.token_expires_at <= Utc::now() {
        return Err(invalid_device_token());
    }
    if row.user_status != "active" {
        return Err(user_disabled());
    }
    if row.device_status != "active" {
        return Err(device_disabled());
    }
    let _ = repo::touch_device(pool, row.device_id, ip).await;
    let _ = repo::touch_token(pool, row.token_id).await;
    let _ = repo::touch_key(pool, row.key_id).await;
    Ok(ClientDeviceResponse {
        device: to_client_device(&row),
        user: User {
            id: row.user_id,
            email: row.user_email,
            display_name: row.user_display_name,
            status: row.user_status,
        },
        key: ClientKeySummary {
            id: row.key_id,
            key_prefix: row.key_prefix,
            status: row.key_status,
            expires_at: row.key_expires_at,
            allowed_models: row.key_allowed_models,
        },
    })
}

// ---------- S4 内部鉴权（S3 先实现并单测，proxy 接入后使用） ----------

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    MissingApiKey,
    MissingDeviceToken,
    /// header 歧义/格式错误（Bearer 与 x-api-key 冲突）；wire code 固定 `invalid_api_key`。
    InvalidHeader,
    InvalidApiKey,
    InvalidDeviceToken,
    DeviceDisabled,
    UserDisabled,
    KeyRevoked,
    KeyExpired,
    KeyDeviceMismatch,
    Database,
}

#[allow(dead_code)]
impl AuthError {
    pub fn code(self) -> &'static str {
        match self {
            Self::MissingApiKey => "missing_api_key",
            Self::MissingDeviceToken => "missing_device_token",
            Self::InvalidHeader => "invalid_api_key",
            Self::InvalidApiKey => "invalid_api_key",
            Self::InvalidDeviceToken => "invalid_device_token",
            Self::DeviceDisabled => "device_disabled",
            Self::UserDisabled => "user_disabled",
            Self::KeyRevoked => "key_revoked",
            Self::KeyExpired => "key_expired",
            Self::KeyDeviceMismatch => "key_device_mismatch",
            Self::Database => "database_unavailable",
        }
    }
}

/// 校验 Key + device_token 同设备、双方 active、Key 未吊销/未过期。
#[allow(dead_code)]
pub async fn authorize(
    state: &AppState,
    api_key: &str,
    device_token: &str,
) -> Result<AuthContext, AuthError> {
    let pool = state.db.as_ref().ok_or(AuthError::Database)?;
    let key_hash = security::sha256_hex(api_key);
    let token_hash = security::sha256_hex(device_token);
    let key = repo::find_key_for_auth(pool, &key_hash)
        .await
        .map_err(|_| AuthError::Database)?
        .ok_or(AuthError::InvalidApiKey)?;
    let token = repo::validate_device_token(pool, &token_hash)
        .await
        .map_err(|_| AuthError::Database)?
        .ok_or(AuthError::InvalidDeviceToken)?;
    if token.token_expires_at <= Utc::now() {
        return Err(AuthError::InvalidDeviceToken);
    }
    if key.user_status != "active" || token.user_status != "active" {
        return Err(AuthError::UserDisabled);
    }
    if key.device_status != "active" || token.device_status != "active" {
        return Err(AuthError::DeviceDisabled);
    }
    if key.device_id != token.device_id || key.key_id != token.key_id {
        return Err(AuthError::KeyDeviceMismatch);
    }
    if key.key_status == "revoked" {
        return Err(AuthError::KeyRevoked);
    }
    if key.key_status != "active" {
        return Err(AuthError::KeyRevoked);
    }
    if key
        .key_expires_at
        .is_some_and(|expires| expires <= Utc::now())
    {
        return Err(AuthError::KeyExpired);
    }
    let user_group_ids = repo::user_group_ids(pool, key.user_id)
        .await
        .map_err(|_| AuthError::Database)?;
    let _ = repo::touch_token(pool, token.token_id).await;
    let _ = repo::touch_key(pool, key.key_id).await;
    Ok(AuthContext {
        user_id: key.user_id,
        device_id: key.device_id,
        key_id: key.key_id,
        allowed_models: key.key_allowed_models,
        key_prefix: Some(key.key_prefix),
        key_rpm_limit: key.key_rpm_limit,
        user_group_ids,
    })
}

/// 数据面 `AuthError` → problem+json 的统一映射（proxy 推理入口与 catalog `/v1/models` 共用）。
///
/// 固定文案，不泄露 Key/设备是否存在；错误码与 proxy 接口设计 §4.2 一致。
pub fn auth_error_to_app_error(error: AuthError) -> AppError {
    match error {
        AuthError::MissingApiKey => AppError::Unauthorized {
            code: "missing_api_key",
            detail: "缺少虚拟 Key（Authorization: Bearer 或 x-api-key）",
        },
        AuthError::MissingDeviceToken => AppError::Unauthorized {
            code: "missing_device_token",
            detail: "缺少设备 token（X-GW-Device-Token）",
        },
        AuthError::InvalidHeader | AuthError::InvalidApiKey => AppError::Unauthorized {
            code: "invalid_api_key",
            detail: "虚拟 Key 无效",
        },
        AuthError::InvalidDeviceToken => AppError::Unauthorized {
            code: "invalid_device_token",
            detail: "设备 token 无效或已过期，请刷新后重试",
        },
        AuthError::KeyRevoked => AppError::Unauthorized {
            code: "key_revoked",
            detail: "Key 已被吊销",
        },
        AuthError::KeyExpired => AppError::Unauthorized {
            code: "key_expired",
            detail: "Key 已过期",
        },
        AuthError::KeyDeviceMismatch => AppError::Unauthorized {
            code: "key_device_mismatch",
            detail: "Key 与设备不匹配",
        },
        AuthError::DeviceDisabled => AppError::Forbidden {
            code: "device_disabled",
            detail: "设备已被禁用或解绑".to_string(),
        },
        AuthError::UserDisabled => AppError::Forbidden {
            code: "user_disabled",
            detail: "用户已被禁用".to_string(),
        },
        AuthError::Database => AppError::database_unavailable(),
    }
}

/// 数据面凭证 header 上限（字节）：超过即视为非法，避免超长凭证反复哈希。
const MAX_CREDENTIAL_HEADER_BYTES: usize = 512;

/// 数据面统一鉴权入口：解析 `Authorization: Bearer`/`x-api-key`/`x-goog-api-key` +
/// `X-GW-Device-Token`，再委托 [`authorize`]（proxy 与 catalog `/v1/models` 共用，
/// 禁止各自复制 401/403 映射）。
///
/// 规则见 proxy 技术设计 §4.1 / Gemini 实现设计 §1：
/// - 多个 Key header 都存在时：值都相等才接受；冲突 → `InvalidHeader`（wire `invalid_api_key`）。
/// - 都没有 → `MissingApiKey`；设备 token 缺失/空 → `MissingDeviceToken`。
/// - 长度 >512 字节或含控制字符 → `InvalidApiKey`/`InvalidDeviceToken`。
#[allow(dead_code)]
pub async fn authenticate_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthContext, AuthError> {
    let api_key = extract_api_key(headers)?;
    let device_token = extract_device_token(headers)?;
    authorize(state, &api_key, &device_token).await
}

fn extract_api_key(headers: &HeaderMap) -> Result<String, AuthError> {
    let bearer = bearer_token(headers)?;
    let x_api_key = header_credential(headers, "x-api-key", AuthError::InvalidApiKey)?;
    // Gemini CLI 使用 `x-goog-api-key`；与其它 Key header 同等，冲突仍 401。
    let x_goog_api_key = header_credential(headers, "x-goog-api-key", AuthError::InvalidApiKey)?;
    let mut found: Option<String> = None;
    for candidate in [bearer, x_api_key, x_goog_api_key].into_iter().flatten() {
        match &found {
            Some(existing) if existing != &candidate => return Err(AuthError::InvalidHeader),
            None => found = Some(candidate),
            Some(_) => {}
        }
    }
    found.ok_or(AuthError::MissingApiKey)
}

/// `Authorization: Bearer <key>`；非 Bearer/空值视为未提供。
fn bearer_token(headers: &HeaderMap) -> Result<Option<String>, AuthError> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| AuthError::InvalidApiKey)?;
    let mut parts = value.splitn(2, char::is_whitespace);
    let scheme = parts.next().unwrap_or_default();
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Ok(None);
    }
    credential_value(parts.next().unwrap_or_default(), AuthError::InvalidApiKey)
}

fn extract_device_token(headers: &HeaderMap) -> Result<String, AuthError> {
    header_credential(headers, "x-gw-device-token", AuthError::InvalidDeviceToken)?
        .ok_or(AuthError::MissingDeviceToken)
}

fn header_credential(
    headers: &HeaderMap,
    name: &'static str,
    invalid: AuthError,
) -> Result<Option<String>, AuthError> {
    let Some(value) = headers.get(name) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| invalid)?;
    credential_value(value, invalid)
}

/// trim 后为空视为未提供；超长/含控制字符视为非法。
fn credential_value(raw: &str, invalid: AuthError) -> Result<Option<String>, AuthError> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_CREDENTIAL_HEADER_BYTES || value.chars().any(char::is_control) {
        return Err(invalid);
    }
    Ok(Some(value.to_string()))
}

#[allow(dead_code)]
pub fn is_model_allowed(context: &AuthContext, model: &str) -> bool {
    context.allowed_models.is_empty() || context.allowed_models.iter().any(|item| item == model)
}

#[cfg(test)]
mod user_group_unit_tests {
    use super::*;

    fn validation_field(error: AppError) -> (String, String) {
        match error {
            AppError::Validation { errors } => errors
                .into_iter()
                .next()
                .expect("validation error has one field"),
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn validates_user_group_name() {
        assert_eq!(
            validate_user_group_name("  Engineering  ").expect("trimmed name"),
            "Engineering"
        );
        assert_eq!(
            validate_user_group_name(&"名".repeat(MAX_USER_GROUP_NAME_CHARS))
                .expect("max chars")
                .chars()
                .count(),
            MAX_USER_GROUP_NAME_CHARS
        );
        assert_eq!(
            validation_field(validate_user_group_name("   ").unwrap_err()).0,
            "name"
        );
        let long = "n".repeat(MAX_USER_GROUP_NAME_CHARS + 1);
        assert_eq!(
            validation_field(validate_user_group_name(&long).unwrap_err()).0,
            "name"
        );
    }

    #[test]
    fn validates_user_group_description() {
        assert_eq!(
            validate_user_group_description(None).expect("absent description"),
            ""
        );
        assert_eq!(
            validate_user_group_description(Some("  core team  ")).expect("trimmed"),
            "core team"
        );
        let long = "d".repeat(MAX_USER_GROUP_DESCRIPTION_CHARS + 1);
        assert_eq!(
            validation_field(validate_user_group_description(Some(&long)).unwrap_err()).0,
            "description"
        );
    }

    #[test]
    fn normalizes_member_ids_dedup_and_limit() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert_eq!(
            normalize_member_ids(&[a, b, a]).expect("dedupe"),
            vec![a, b]
        );
        assert!(normalize_member_ids(&[]).expect("empty").is_empty());

        let max: Vec<Uuid> = (0..MAX_USER_GROUP_MEMBERS)
            .map(|_| Uuid::new_v4())
            .collect();
        assert_eq!(
            normalize_member_ids(&max).expect("max members").len(),
            MAX_USER_GROUP_MEMBERS
        );
        // 重复项不计入上限：501 个原始元素、去重后 500 → 允许。
        let mut with_duplicate = max.clone();
        with_duplicate.push(max[0]);
        assert_eq!(
            normalize_member_ids(&with_duplicate)
                .expect("duplicates within limit")
                .len(),
            MAX_USER_GROUP_MEMBERS
        );
        let over: Vec<Uuid> = (0..=MAX_USER_GROUP_MEMBERS)
            .map(|_| Uuid::new_v4())
            .collect();
        assert_eq!(
            validation_field(normalize_member_ids(&over).unwrap_err()).0,
            "member_ids"
        );
    }

    #[test]
    fn user_group_query_defaults_trim_and_limits() {
        let query = UserGroupQuery {
            q: None,
            limit: None,
            offset: None,
        };
        assert_eq!(
            user_group_query_params(&query).expect("defaults"),
            (None, DEFAULT_PAGE_LIMIT, 0)
        );

        let query = UserGroupQuery {
            q: Some("  eng  ".to_string()),
            limit: Some(10),
            offset: Some(5),
        };
        assert_eq!(
            user_group_query_params(&query).expect("explicit params"),
            (Some("eng".to_string()), 10, 5)
        );

        let query = UserGroupQuery {
            q: Some("   ".to_string()),
            limit: None,
            offset: None,
        };
        assert_eq!(user_group_query_params(&query).expect("blank q").0, None);

        for (limit, offset, field) in [
            (Some(0), None, "limit"),
            (Some(MAX_PAGE_LIMIT + 1), None, "limit"),
            (None, Some(-1), "offset"),
        ] {
            let query = UserGroupQuery {
                q: None,
                limit,
                offset,
            };
            assert_eq!(
                validation_field(user_group_query_params(&query).unwrap_err()).0,
                field
            );
        }
    }

    #[test]
    fn user_group_not_found_code() {
        assert!(matches!(
            user_group_not_found(),
            AppError::NotFound {
                code: "user_group_not_found",
                ..
            }
        ));
    }
}

#[cfg(test)]
mod header_auth_tests {
    use super::*;
    use crate::platform::{config::Config, crypto::Crypto};
    use axum::http::{HeaderName, HeaderValue};
    use std::sync::Arc;

    /// 无数据库的 AppState：合法 header 解析后会落到 authorize 的 Database 分支，
    /// 用于验证“header 解析已通过”。
    fn state_without_db() -> AppState {
        let config = Config {
            bind: "127.0.0.1:0".parse().expect("test bind addr"),
            database_url: None,
            log_filter: "off".to_string(),
            run_migrations: false,
            cookie_secure: false,
            setup_token: None,
            usd_cny_rate: rust_decimal::Decimal::new(720, 2),
            trust_proxy_headers: false,
            proxy_max_body_bytes: 8 * 1024 * 1024,
            proxy_expose_provider_header: false,
            usage_retention_days: 180,
            rollup_retention_days: 1825,
            rollup_interval_secs: 300,
            request_log_retention_days: 30,
            audit_retention_days: 180,
            log_cleanup_interval_secs: 3600,
        };
        AppState::new(
            Arc::new(config),
            None,
            Arc::new(Crypto::from_key_bytes([3u8; 32])),
        )
    }

    fn headers(pairs: &[(HeaderName, HeaderValue)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(name.clone(), value.clone());
        }
        map
    }

    fn bearer(value: &str) -> (HeaderName, HeaderValue) {
        (
            header::AUTHORIZATION,
            value.parse().expect("authorization header"),
        )
    }

    fn x_api_key(value: &str) -> (HeaderName, HeaderValue) {
        (
            HeaderName::from_static("x-api-key"),
            value.parse().expect("x-api-key header"),
        )
    }

    fn x_goog_api_key(value: &str) -> (HeaderName, HeaderValue) {
        (
            HeaderName::from_static("x-goog-api-key"),
            value.parse().expect("x-goog-api-key header"),
        )
    }

    fn device_token(value: &str) -> (HeaderName, HeaderValue) {
        (
            HeaderName::from_static("x-gw-device-token"),
            value.parse().expect("device token header"),
        )
    }

    #[tokio::test]
    async fn missing_api_key_is_reported() {
        let state = state_without_db();
        assert!(matches!(
            authenticate_headers(&state, &HeaderMap::new()).await,
            Err(AuthError::MissingApiKey)
        ));
        // 只有设备 token 也不行（双因子）。
        let map = headers(&[device_token("dev-token")]);
        assert!(matches!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::MissingApiKey)
        ));
        // 空的 Bearer 视为未提供。
        let map = headers(&[bearer("Bearer   "), device_token("dev-token")]);
        assert!(matches!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::MissingApiKey)
        ));
    }

    #[tokio::test]
    async fn missing_device_token_is_reported() {
        let state = state_without_db();
        let map = headers(&[bearer("Bearer sk-gw-abc")]);
        assert!(matches!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::MissingDeviceToken)
        ));
        // 空 token 视为缺失。
        let map = headers(&[bearer("Bearer sk-gw-abc"), device_token("   ")]);
        assert!(matches!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::MissingDeviceToken)
        ));
    }

    #[tokio::test]
    async fn bearer_and_x_api_key_are_equivalent() {
        let state = state_without_db();
        // 相同 → 解析通过，落到 authorize（无 DB → Database）。
        let map = headers(&[
            bearer("Bearer sk-gw-same"),
            x_api_key("sk-gw-same"),
            device_token("dev-token"),
        ]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::Database)
        );
        // 只有 x-api-key。
        let map = headers(&[x_api_key("sk-gw-same"), device_token("dev-token")]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::Database)
        );
        // Bearer 方案大小写不敏感，token 前后空白被 trim。
        let map = headers(&[bearer("bearer  sk-gw-same  "), device_token("dev-token")]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::Database)
        );
        // Gemini CLI 的 x-goog-api-key 与 Bearer/x-api-key 等价。
        let map = headers(&[x_goog_api_key("sk-gw-same"), device_token("dev-token")]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::Database)
        );
        let map = headers(&[
            bearer("Bearer sk-gw-same"),
            x_api_key("sk-gw-same"),
            x_goog_api_key("sk-gw-same"),
            device_token("dev-token"),
        ]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::Database)
        );
        // 非 Bearer 的 Authorization 被忽略。
        let map = headers(&[bearer("Basic sk-gw-same"), device_token("dev-token")]);
        assert!(matches!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::MissingApiKey)
        ));
    }

    #[tokio::test]
    async fn conflicting_key_headers_are_rejected() {
        let state = state_without_db();
        let map = headers(&[
            bearer("Bearer sk-gw-a"),
            x_api_key("sk-gw-b"),
            device_token("dev-token"),
        ]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::InvalidHeader)
        );
        assert_eq!(AuthError::InvalidHeader.code(), "invalid_api_key");
        // x-goog-api-key 与其它 Key header 冲突同样 401。
        let map = headers(&[
            bearer("Bearer sk-gw-a"),
            x_goog_api_key("sk-gw-b"),
            device_token("dev-token"),
        ]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::InvalidHeader)
        );
    }

    #[tokio::test]
    async fn oversize_or_control_char_credentials_are_rejected() {
        let state = state_without_db();
        let long = "k".repeat(MAX_CREDENTIAL_HEADER_BYTES + 1);
        let map = headers(&[bearer(&format!("Bearer {long}")), device_token("dev-token")]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::InvalidApiKey)
        );
        let map = headers(&[x_api_key(&long), device_token("dev-token")]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::InvalidApiKey)
        );
        let map = headers(&[
            bearer("Bearer sk-gw-abc"),
            (
                HeaderName::from_static("x-gw-device-token"),
                HeaderValue::from_bytes(&vec![b't'; MAX_CREDENTIAL_HEADER_BYTES + 1])
                    .expect("long token header"),
            ),
        ]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::InvalidDeviceToken)
        );
        // 控制字符（HTAB 在 header 值中合法，但凭证不允许）。
        let map = headers(&[
            (
                header::AUTHORIZATION,
                HeaderValue::from_bytes(b"Bearer sk\tgw").expect("tab key header"),
            ),
            device_token("dev-token"),
        ]);
        assert_eq!(
            authenticate_headers(&state, &map).await,
            Err(AuthError::InvalidApiKey)
        );
    }

    #[test]
    fn auth_error_codes_for_header_errors() {
        assert_eq!(AuthError::MissingApiKey.code(), "missing_api_key");
        assert_eq!(AuthError::MissingDeviceToken.code(), "missing_device_token");
        assert_eq!(AuthError::InvalidApiKey.code(), "invalid_api_key");
        assert_eq!(AuthError::InvalidDeviceToken.code(), "invalid_device_token");
        assert_eq!(AuthError::Database.code(), "database_unavailable");
    }
}

#[cfg(test)]
mod db_flow_tests {
    use super::*;
    use crate::platform::{config::Config, crypto::Crypto};
    use std::{sync::Arc, time::Instant};

    async fn test_state() -> Option<AppState> {
        let url = std::env::var("GW_TEST_DATABASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())?;
        let pool = sqlx::PgPool::connect(&url)
            .await
            .expect("connect GW_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        sqlx::query(
            "TRUNCATE users, devices, pairing_codes, device_tokens, virtual_keys, user_groups CASCADE",
        )
            .execute(&pool)
            .await
            .expect("truncate access tables");
        let config = Config {
            bind: "127.0.0.1:0".parse().expect("test bind addr"),
            database_url: Some(url),
            log_filter: "off".to_string(),
            run_migrations: false,
            cookie_secure: false,
            setup_token: None,
            usd_cny_rate: rust_decimal::Decimal::new(720, 2),
            trust_proxy_headers: false,
            proxy_max_body_bytes: 8 * 1024 * 1024,
            proxy_expose_provider_header: false,
            usage_retention_days: 180,
            rollup_retention_days: 1825,
            rollup_interval_secs: 300,
            request_log_retention_days: 30,
            audit_retention_days: 180,
            log_cleanup_interval_secs: 3600,
        };
        Some(AppState {
            config: Arc::new(config),
            db: Some(pool),
            crypto: Arc::new(Crypto::from_key_bytes([4u8; 32])),
            http: crate::platform::http::shared_client(),
            started_at: Instant::now(),
        })
    }

    async fn admin_id(state: &AppState) -> Uuid {
        let pool = db(state).expect("test pool");
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO admin_users (id, email, password_hash, display_name) \
             VALUES ($1, 'access-test-admin@corp.example.com', 'test-hash', 'Admin')",
        )
        .bind(id)
        .execute(pool)
        .await
        .expect("insert admin");
        id
    }

    /// 插入唯一邮箱管理员（admin_users 不在 access TRUNCATE 范围内，避免跨 gated 用例冲突）。
    async fn unique_admin_id(state: &AppState) -> Uuid {
        let pool = db(state).expect("test pool");
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO admin_users (id, email, password_hash, display_name) \
             VALUES ($1, $2, 'test-hash', 'Admin')",
        )
        .bind(id)
        .bind(format!("access-ug-admin-{id}@corp.example.com"))
        .execute(pool)
        .await
        .expect("insert admin");
        id
    }

    async fn seed_user(state: &AppState, email: &str, status: &str) -> Uuid {
        let pool = db(state).expect("test pool");
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, email, display_name, status) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(email)
            .bind(format!("用户 {email}"))
            .bind(status)
            .execute(pool)
            .await
            .expect("insert user");
        id
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库（会 TRUNCATE access 表）"]
    async fn access_full_flow_and_authorize() {
        let Some(state) = test_state().await else {
            return;
        };
        let pool = db(&state).expect("pool");
        let admin = admin_id(&state).await;

        let created = create_pairing_code(
            &state,
            admin,
            CreatePairingCodeRequest {
                user_email: "alice@corp.example.com".to_string(),
                user_display_name: Some("Alice".to_string()),
                expires_in_seconds: None,
            },
        )
        .await
        .expect("create pairing code");
        let code = created.pairing_code.code.clone();
        assert_eq!(created.pairing_code.meta.user.display_name, "Alice");

        let paired = pair_client(
            &state,
            ClientPairRequest {
                code,
                device: Some(DeviceInput {
                    name: Some("Alice Mac".to_string()),
                    platform: Some("macos".to_string()),
                    client_version: Some("0.1.0".to_string()),
                }),
            },
            "127.0.0.2",
        )
        .await
        .expect("pair");
        assert_eq!(paired.user.email, "alice@corp.example.com");
        assert_eq!(paired.device_secret.len(), 43);
        assert_eq!(paired.api_key_prefix, paired.api_key[..12]);
        let api_key_prefix = paired.api_key_prefix.clone();
        let device_id = paired.device.id;
        let secret = paired.device_secret.clone();
        let api_key = paired.api_key.clone();
        let token = paired.token.clone();

        // DB 不存明文（secret/token/key/code），哈希 64 hex。
        let device_secret_hash: String =
            sqlx::query_scalar("SELECT device_secret_hash FROM devices WHERE id = $1")
                .bind(device_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(device_secret_hash.len(), 64);
        assert!(!device_secret_hash.contains(&secret));
        let key_hash: String =
            sqlx::query_scalar("SELECT key_hash FROM virtual_keys WHERE device_id = $1")
                .bind(device_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(key_hash.len(), 64);
        assert!(!key_hash.contains(&api_key));
        let token_hash: String =
            sqlx::query_scalar("SELECT token_hash FROM device_tokens WHERE device_id = $1 LIMIT 1")
                .bind(device_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(token_hash.len(), 64);
        assert!(!token_hash.contains(&token));
        let code_hash: String = sqlx::query_scalar(
            "SELECT code_hash FROM pairing_codes WHERE user_id = \
             (SELECT id FROM users WHERE email = 'alice@corp.example.com')",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(code_hash.len(), 64);

        // 内部双因子鉴权。
        let context = authorize(&state, &api_key, &token)
            .await
            .expect("authorize");
        assert_eq!(context.device_id, device_id);
        assert!(is_model_allowed(&context, "any-model"));
        assert!(context.user_group_ids.is_empty());

        // authenticate_headers：Bearer / x-api-key 等价，key_prefix 写入上下文。
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {api_key}").parse().expect("bearer header"),
        );
        headers.insert(
            "x-gw-device-token",
            token.parse().expect("device token header"),
        );
        let context = authenticate_headers(&state, &headers)
            .await
            .expect("authenticate bearer headers");
        assert_eq!(context.device_id, device_id);
        assert_eq!(context.key_prefix.as_deref(), Some(api_key_prefix.as_str()));

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", api_key.parse().expect("x-api-key header"));
        headers.insert(
            "x-gw-device-token",
            token.parse().expect("device token header"),
        );
        let context = authenticate_headers(&state, &headers)
            .await
            .expect("authenticate x-api-key headers");
        assert_eq!(context.device_id, device_id);
        assert_eq!(context.key_prefix.as_deref(), Some(api_key_prefix.as_str()));

        // 两个 Key header 冲突 → invalid_api_key（401）。
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {api_key}").parse().expect("bearer header"),
        );
        headers.insert(
            "x-api-key",
            "sk-gw-other".parse().expect("x-api-key header"),
        );
        headers.insert(
            "x-gw-device-token",
            token.parse().expect("device token header"),
        );
        assert_eq!(
            authenticate_headers(&state, &headers).await,
            Err(AuthError::InvalidHeader)
        );
        assert_eq!(AuthError::InvalidHeader.code(), "invalid_api_key");

        // 白名单。
        update_key(
            &state,
            context.key_id,
            PatchKeyRequest {
                allowed_models: Some(vec!["claude-sonnet-4-5".to_string()]),
                expires_at: None,
                rpm_limit: None,
            },
        )
        .await
        .expect("update key allowlist");
        let context = authorize(&state, &api_key, &token)
            .await
            .expect("authorize");
        assert!(is_model_allowed(&context, "claude-sonnet-4-5"));
        assert!(!is_model_allowed(&context, "gpt-5"));

        // S3.1 Key RPM：0011 迁移列/CHECK、PATCH 设置/null 清除/范围校验/KeyAdmin/AuthContext。
        let pool = db(&state).expect("pool");
        let has_rpm_column: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM information_schema.columns              WHERE table_name = 'virtual_keys' AND column_name = 'rpm_limit')",
        )
        .fetch_one(pool)
        .await
        .expect("rpm column");
        assert!(has_rpm_column);
        let invalid_db = sqlx::query("UPDATE virtual_keys SET rpm_limit = 0 WHERE id = $1")
            .bind(context.key_id)
            .execute(pool)
            .await;
        assert!(invalid_db.is_err(), "DB CHECK must reject rpm_limit=0");

        let updated = update_key(
            &state,
            context.key_id,
            PatchKeyRequest {
                allowed_models: None,
                expires_at: None,
                rpm_limit: Some(Some(2)),
            },
        )
        .await
        .expect("set rpm");
        assert_eq!(updated.rpm_limit, Some(2));
        let fetched = get_key(&state, context.key_id).await.expect("get key");
        assert_eq!(fetched.rpm_limit, Some(2));
        let listed = list_keys(
            &state,
            &PageQuery {
                status: None,
                user_id: None,
                device_id: None,
                limit: None,
                offset: None,
            },
        )
        .await
        .expect("list keys");
        assert_eq!(listed.items[0].rpm_limit, Some(2));
        let context = authorize(&state, &api_key, &token)
            .await
            .expect("authorize rpm");
        assert_eq!(context.key_rpm_limit, Some(2));

        let cleared = update_key(
            &state,
            context.key_id,
            PatchKeyRequest {
                allowed_models: None,
                expires_at: None,
                rpm_limit: Some(None),
            },
        )
        .await
        .expect("clear rpm");
        assert_eq!(cleared.rpm_limit, None);
        assert_eq!(
            authorize(&state, &api_key, &token)
                .await
                .expect("authorize cleared")
                .key_rpm_limit,
            None
        );

        for invalid in [0, -1, 1_000_001] {
            assert!(matches!(
                update_key(
                    &state,
                    context.key_id,
                    PatchKeyRequest {
                        allowed_models: None,
                        expires_at: None,
                        rpm_limit: Some(Some(invalid)),
                    },
                )
                .await,
                Err(AppError::Validation { errors }) if errors.contains_key("rpm_limit")
            ));
        }
        assert!(matches!(
            update_key(
                &state,
                context.key_id,
                PatchKeyRequest {
                    allowed_models: None,
                    expires_at: None,
                    rpm_limit: None,
                },
            )
            .await,
            Err(AppError::Validation { errors }) if errors.contains_key("body")
        ));

        // 吊销 → 内部鉴权失败；重复吊销幂等。
        revoke_key(&state, context.key_id).await.expect("revoke");
        revoke_key(&state, context.key_id)
            .await
            .expect("revoke idempotent");
        assert!(matches!(
            authorize(&state, &api_key, &token).await,
            Err(AuthError::KeyRevoked)
        ));

        // 重置 → 旧 Key 失效、token 被删；新 Key 可鉴权（重新换 token）。
        let reset = reset_key(&state, context.key_id).await.expect("reset");
        assert_ne!(reset.api_key, api_key);
        assert!(matches!(
            authorize(&state, &api_key, &token).await,
            Err(AuthError::InvalidApiKey)
        ));
        let token = exchange_token(
            &state,
            TokenRequest {
                device_id,
                device_secret: secret.clone(),
            },
            "127.0.0.2",
        )
        .await
        .expect("exchange")
        .token;
        let context = authorize(&state, &reset.api_key, &token)
            .await
            .expect("authorize reset key");
        assert_eq!(context.key_id, reset.key.id);

        // 设备禁用/启用：token 删除、Key paused/active。
        let disabled = update_device(
            &state,
            device_id,
            PatchDeviceRequest {
                name: None,
                status: Some("disabled".to_string()),
            },
        )
        .await
        .expect("disable device");
        assert_eq!(disabled.status, "disabled");
        assert_eq!(
            disabled.key.as_ref().map(|key| key.status.as_str()),
            Some("paused")
        );
        let exchange_error = exchange_token(
            &state,
            TokenRequest {
                device_id,
                device_secret: secret.clone(),
            },
            "127.0.0.2",
        )
        .await
        .expect_err("disabled device cannot refresh");
        assert!(matches!(
            exchange_error,
            AppError::Forbidden {
                code: "device_disabled",
                ..
            }
        ));
        let enabled = update_device(
            &state,
            device_id,
            PatchDeviceRequest {
                name: None,
                status: Some("active".to_string()),
            },
        )
        .await
        .expect("enable device");
        assert_eq!(
            enabled.key.as_ref().map(|key| key.status.as_str()),
            Some("active")
        );

        // 用户禁用 → token 删除、换 token 403。
        let user = repo::list_users(pool, None, 10, 0)
            .await
            .expect("list users")
            .0
            .into_iter()
            .next()
            .expect("user");
        update_user(
            &state,
            user.id,
            PatchUserRequest {
                display_name: None,
                status: Some("disabled".to_string()),
            },
        )
        .await
        .expect("disable user");
        let exchange_error = exchange_token(
            &state,
            TokenRequest {
                device_id,
                device_secret: secret.clone(),
            },
            "127.0.0.2",
        )
        .await
        .expect_err("disabled user cannot refresh");
        assert!(matches!(
            exchange_error,
            AppError::Forbidden {
                code: "user_disabled",
                ..
            }
        ));
        update_user(
            &state,
            user.id,
            PatchUserRequest {
                display_name: Some("Alice Updated".to_string()),
                status: Some("active".to_string()),
            },
        )
        .await
        .expect("enable user");

        // 解绑：设备 disabled、Key revoked。
        unbind_device(&state, device_id).await.expect("unbind");
        let key = get_key(&state, context.key_id).await.expect("get key");
        assert_eq!(key.status, "revoked");
        assert!(key.revoked_at.is_some());

        // 并发同码：仅一个成功。
        let second = create_pairing_code(
            &state,
            admin,
            CreatePairingCodeRequest {
                user_email: "bob@corp.example.com".to_string(),
                user_display_name: None,
                expires_in_seconds: None,
            },
        )
        .await
        .expect("second code");
        let second_code = second.pairing_code.code;
        let state_a = state.clone();
        let state_b = state.clone();
        let code_a = second_code.clone();
        let code_b = second_code;
        let (first, second_result) = tokio::join!(
            pair_client(
                &state_a,
                ClientPairRequest {
                    code: code_a,
                    device: None,
                },
                "127.0.0.3"
            ),
            pair_client(
                &state_b,
                ClientPairRequest {
                    code: code_b,
                    device: None,
                },
                "127.0.0.4"
            )
        );
        let successes = usize::from(first.is_ok()) + usize::from(second_result.is_ok());
        assert_eq!(successes, 1, "exactly one concurrent pair must succeed");
        let failed = if first.is_err() { first } else { second_result };
        assert!(matches!(
            failed.expect_err("one failure"),
            AppError::Unauthorized {
                code: "pairing_code_invalid",
                ..
            }
        ));
    }

    async fn count_group_members(pool: &sqlx::PgPool, group_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM user_group_members WHERE group_id = $1")
            .bind(group_id)
            .fetch_one(pool)
            .await
            .expect("count group members")
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库（会 TRUNCATE access 表）"]
    async fn user_groups_crud_members_and_auth_context() {
        let Some(state) = test_state().await else {
            return;
        };
        let pool = db(&state).expect("pool");
        let user_a = seed_user(&state, "ug-a@corp.example.com", "active").await;
        let user_b = seed_user(&state, "ug-b@corp.example.com", "active").await;
        let user_disabled = seed_user(&state, "ug-disabled@corp.example.com", "disabled").await;

        // create：name/description trim；member_ids 去重。
        let created = create_user_group(
            &state,
            CreateUserGroupRequest {
                name: "  Engineering  ".to_string(),
                description: Some("  core team  ".to_string()),
                member_ids: Some(vec![user_a, user_b, user_a]),
            },
        )
        .await
        .expect("create engineering");
        assert_eq!(created.name, "Engineering");
        assert_eq!(created.description, "core team");
        assert_eq!(created.members.len(), 2);
        let engineering_members: Vec<Uuid> = created.members.iter().map(|user| user.id).collect();
        assert!(engineering_members.contains(&user_a) && engineering_members.contains(&user_b));
        let engineering_id = created.id;

        // create：无成员 + disabled 用户允许入组（实现评审 Medium）。
        let platform = create_user_group(
            &state,
            CreateUserGroupRequest {
                name: "Platform".to_string(),
                description: None,
                member_ids: Some(vec![user_disabled]),
            },
        )
        .await
        .expect("create platform");
        assert_eq!(platform.members.len(), 1);
        assert_eq!(platform.members[0].status, "disabled");
        assert_eq!(platform.description, "");
        let platform_id = platform.id;

        // 名称大小写不敏感唯一 → 409。
        let duplicate = create_user_group(
            &state,
            CreateUserGroupRequest {
                name: "engineering".to_string(),
                description: None,
                member_ids: None,
            },
        )
        .await
        .expect_err("duplicate name");
        assert!(matches!(
            duplicate,
            AppError::Conflict {
                code: "user_group_name_conflict",
                ..
            }
        ));

        // 校验：空/超长名称、超长描述、成员用户不存在（失败请求不落库）。
        for invalid in [
            create_user_group(
                &state,
                CreateUserGroupRequest {
                    name: "   ".to_string(),
                    description: None,
                    member_ids: None,
                },
            )
            .await
            .expect_err("empty name"),
            create_user_group(
                &state,
                CreateUserGroupRequest {
                    name: "n".repeat(MAX_USER_GROUP_NAME_CHARS + 1),
                    description: None,
                    member_ids: None,
                },
            )
            .await
            .expect_err("long name"),
            create_user_group(
                &state,
                CreateUserGroupRequest {
                    name: "Valid".to_string(),
                    description: Some("d".repeat(MAX_USER_GROUP_DESCRIPTION_CHARS + 1)),
                    member_ids: None,
                },
            )
            .await
            .expect_err("long description"),
        ] {
            assert!(matches!(invalid, AppError::Validation { .. }));
        }
        let missing_member = create_user_group(
            &state,
            CreateUserGroupRequest {
                name: "Ghost".to_string(),
                description: None,
                member_ids: Some(vec![Uuid::new_v4()]),
            },
        )
        .await
        .expect_err("missing member");
        assert!(matches!(
            missing_member,
            AppError::NotFound {
                code: "user_not_found",
                ..
            }
        ));

        // list：member_count / updated_at DESC / q / 分页。
        let page = list_user_groups(
            &state,
            &UserGroupQuery {
                q: None,
                limit: None,
                offset: None,
            },
        )
        .await
        .expect("list user groups");
        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].id, platform_id, "updated_at DESC");
        let engineering_summary = page
            .items
            .iter()
            .find(|item| item.id == engineering_id)
            .expect("engineering summary");
        assert_eq!(engineering_summary.member_count, 2);
        let platform_summary = page
            .items
            .iter()
            .find(|item| item.id == platform_id)
            .expect("platform summary");
        assert_eq!(platform_summary.member_count, 1);

        for (q, expected) in [("  ENG  ", 1), ("CORE", 1), ("zzz", 0)] {
            let filtered = list_user_groups(
                &state,
                &UserGroupQuery {
                    q: Some(q.to_string()),
                    limit: None,
                    offset: None,
                },
            )
            .await
            .expect("filtered list");
            assert_eq!(filtered.total, expected, "q={q}");
        }
        let filtered = list_user_groups(
            &state,
            &UserGroupQuery {
                q: Some("ENG".to_string()),
                limit: None,
                offset: None,
            },
        )
        .await
        .expect("q name");
        assert_eq!(filtered.items[0].id, engineering_id);

        let page_two = list_user_groups(
            &state,
            &UserGroupQuery {
                q: None,
                limit: Some(1),
                offset: Some(1),
            },
        )
        .await
        .expect("page 2");
        assert_eq!(page_two.total, 2);
        assert_eq!(page_two.items.len(), 1);
        assert_eq!((page_two.limit, page_two.offset), (1, 1));
        assert_eq!(page_two.items[0].id, engineering_id);
        assert!(matches!(
            list_user_groups(
                &state,
                &UserGroupQuery {
                    q: None,
                    limit: Some(0),
                    offset: None,
                }
            )
            .await,
            Err(AppError::Validation { .. })
        ));

        // get：详情 / 不存在。
        let detail = get_user_group_detail(&state, engineering_id)
            .await
            .expect("get detail");
        assert_eq!(detail.members.len(), 2);
        assert!(matches!(
            get_user_group_detail(&state, Uuid::new_v4()).await,
            Err(AppError::NotFound {
                code: "user_group_not_found",
                ..
            })
        ));

        // patch：成员全量替换（去重）、description null 清空、name 更新。
        let patched = update_user_group(
            &state,
            engineering_id,
            UpdateUserGroupRequest {
                name: Some("  Engineering Core  ".to_string()),
                description: Some(None),
                member_ids: Some(vec![user_b, user_disabled, user_b]),
            },
        )
        .await
        .expect("patch group");
        assert_eq!(patched.name, "Engineering Core");
        assert_eq!(patched.description, "");
        let patched_members: Vec<Uuid> = patched.members.iter().map(|user| user.id).collect();
        assert_eq!(patched_members.len(), 2);
        assert!(
            patched_members.contains(&user_b)
                && patched_members.contains(&user_disabled)
                && !patched_members.contains(&user_a)
        );

        // patch 空 body → 400；只改名称时成员保持不变。
        assert!(matches!(
            update_user_group(
                &state,
                engineering_id,
                UpdateUserGroupRequest {
                    name: None,
                    description: None,
                    member_ids: None,
                }
            )
            .await,
            Err(AppError::Validation { .. })
        ));
        let renamed = update_user_group(
            &state,
            engineering_id,
            UpdateUserGroupRequest {
                name: Some("Engineering Core Team".to_string()),
                description: Some(Some("core".to_string())),
                member_ids: None,
            },
        )
        .await
        .expect("rename");
        assert_eq!(renamed.members.len(), 2);
        assert_eq!(renamed.description, "core");
        // 更新名称冲突 → 409。
        let rename_conflict = update_user_group(
            &state,
            platform_id,
            UpdateUserGroupRequest {
                name: Some("engineering core team".to_string()),
                description: None,
                member_ids: None,
            },
        )
        .await
        .expect_err("rename conflict");
        assert!(matches!(
            rename_conflict,
            AppError::Conflict {
                code: "user_group_name_conflict",
                ..
            }
        ));
        // member_ids=[] → 清空；成员不存在 → 404 且事务回滚。
        let cleared = update_user_group(
            &state,
            engineering_id,
            UpdateUserGroupRequest {
                name: None,
                description: None,
                member_ids: Some(Vec::new()),
            },
        )
        .await
        .expect("clear members");
        assert!(cleared.members.is_empty());
        let missing_on_patch = update_user_group(
            &state,
            engineering_id,
            UpdateUserGroupRequest {
                name: None,
                description: None,
                member_ids: Some(vec![Uuid::new_v4()]),
            },
        )
        .await
        .expect_err("missing member on patch");
        assert!(matches!(
            missing_on_patch,
            AppError::NotFound {
                code: "user_not_found",
                ..
            }
        ));
        assert!(
            get_user_group_detail(&state, engineering_id)
                .await
                .expect("detail after rollback")
                .members
                .is_empty()
        );

        // 内部 helpers（catalog 下一轮使用）。
        update_user_group(
            &state,
            engineering_id,
            UpdateUserGroupRequest {
                name: None,
                description: None,
                member_ids: Some(vec![user_b]),
            },
        )
        .await
        .expect("set member for helpers");
        assert_eq!(
            user_group_ids(&state, user_b).await.expect("user groups"),
            vec![engineering_id]
        );
        assert!(
            user_group_ids(&state, user_a)
                .await
                .expect("no groups")
                .is_empty()
        );
        assert!(
            user_group_exists(&state, engineering_id)
                .await
                .expect("group exists")
        );
        assert!(
            !user_group_exists(&state, Uuid::new_v4())
                .await
                .expect("group missing")
        );

        // AuthContext.user_group_ids：配对后 authorize / authenticate_headers 返回所属组。
        let admin = unique_admin_id(&state).await;
        let paired = pair_client(
            &state,
            ClientPairRequest {
                code: create_pairing_code(
                    &state,
                    admin,
                    CreatePairingCodeRequest {
                        user_email: "ug-b@corp.example.com".to_string(),
                        user_display_name: None,
                        expires_in_seconds: None,
                    },
                )
                .await
                .expect("pairing code for ug-b")
                .pairing_code
                .code,
                device: None,
            },
            "127.0.0.5",
        )
        .await
        .expect("pair ug-b");
        assert_eq!(paired.user.id, user_b);
        let context = authorize(&state, &paired.api_key, &paired.token)
            .await
            .expect("authorize grouped user");
        assert_eq!(context.user_group_ids, vec![engineering_id]);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", paired.api_key)
                .parse()
                .expect("bearer header"),
        );
        headers.insert(
            "x-gw-device-token",
            paired.token.parse().expect("device token header"),
        );
        let context = authenticate_headers(&state, &headers)
            .await
            .expect("authenticate grouped user");
        assert_eq!(context.user_group_ids, vec![engineering_id]);

        // delete：级联成员、幂等（不存在也成功）。
        assert_eq!(count_group_members(pool, platform_id).await, 1);
        delete_user_group(&state, platform_id)
            .await
            .expect("delete platform");
        assert_eq!(count_group_members(pool, platform_id).await, 0);
        assert!(matches!(
            get_user_group_detail(&state, platform_id).await,
            Err(AppError::NotFound { .. })
        ));
        delete_user_group(&state, platform_id)
            .await
            .expect("delete platform idempotent");
        delete_user_group(&state, Uuid::new_v4())
            .await
            .expect("delete missing idempotent");
        assert_eq!(
            list_user_groups(
                &state,
                &UserGroupQuery {
                    q: None,
                    limit: None,
                    offset: None,
                }
            )
            .await
            .expect("list after delete")
            .total,
            1
        );

        // 删除组后 AuthContext 不再包含该组。
        delete_user_group(&state, engineering_id)
            .await
            .expect("delete engineering");
        let context = authorize(&state, &paired.api_key, &paired.token)
            .await
            .expect("authorize after delete");
        assert!(context.user_group_ids.is_empty());
    }
}
