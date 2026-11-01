//! settings 用例：启动加载、GET、PATCH（单事务持久化 + runtime 生效）。
//!
//! 设计见 `设计文档（暂未开源）` §2/§3：
//! DB `app_settings` 单例为权威；PATCH 校验范围/默认供应商存在性，事务提交后更新 runtime，
//! 供 observability 清理与 proxy 超时读取（下一 tick / 下一请求生效）。

use crate::modules::providers;
use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{
        MAX_AUDIT_RETENTION_DAYS, MAX_REQUEST_LOG_RETENTION_DAYS, MAX_UPSTREAM_TIMEOUT_SECS,
        MIN_AUDIT_RETENTION_DAYS, MIN_REQUEST_LOG_RETENTION_DAYS, MIN_UPSTREAM_TIMEOUT_SECS,
        PatchSettingsRequest, Settings,
    },
    repo, runtime,
};

fn db(state: &AppState) -> Result<&sqlx::PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
}

/// 启动加载 `app_settings` → runtime；DB 不可用/缺行 warn 后使用 env/默认兜底。
pub async fn initialize(state: &AppState) {
    let Some(pool) = state.db.as_ref() else {
        tracing::warn!("app_settings unavailable (database disabled); using env/default settings");
        runtime::set(runtime::fallback_from_config(&state.config));
        return;
    };
    match repo::get_settings(pool).await {
        Ok(Some(row)) => {
            let settings = to_settings(row);
            tracing::info!(
                request_log_retention_days = settings.request_log_retention_days,
                audit_retention_days = settings.audit_retention_days,
                upstream_timeout_secs = settings.upstream_timeout_secs,
                default_provider_id = ?settings.default_provider_id,
                "app_settings loaded"
            );
            runtime::set(settings);
        }
        Ok(None) => {
            tracing::warn!("app_settings row missing; using env/default settings");
            runtime::set(runtime::fallback_from_config(&state.config));
        }
        Err(error) => {
            tracing::warn!(%error, "load app_settings failed; using env/default settings");
            runtime::set(runtime::fallback_from_config(&state.config));
        }
    }
}

/// `GET /api/admin/settings`：读取单例设置（DB 权威）。
pub async fn get_settings(state: &AppState) -> Result<Settings, AppError> {
    let pool = db(state)?;
    let row = repo::get_settings(pool)
        .await?
        .ok_or_else(|| AppError::internal("app_settings singleton missing"))?;
    Ok(to_settings(row))
}

/// `PATCH /api/admin/settings`：部分字段更新（至少一字段），事务提交后 runtime 立即生效。
pub async fn patch_settings(
    state: &AppState,
    patch: PatchSettingsRequest,
) -> Result<Settings, AppError> {
    validate_patch(&patch)?;
    // 默认供应商必须存在（404 provider_not_found）；null = 清除。
    if let Some(Some(provider_id)) = patch.default_provider_id {
        providers::service::get_provider(state, provider_id).await?;
    }

    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    let mut row = repo::lock_settings(&mut tx)
        .await?
        .ok_or_else(|| AppError::internal("app_settings singleton missing; run migrations"))?;
    if let Some(default_provider_id) = patch.default_provider_id {
        row.default_provider_id = default_provider_id;
    }
    if let Some(value) = patch.request_log_retention_days {
        row.request_log_retention_days = value;
    }
    if let Some(value) = patch.audit_retention_days {
        row.audit_retention_days = value;
    }
    if let Some(value) = patch.upstream_timeout_secs {
        row.upstream_timeout_secs = value;
    }
    let updated = repo::update_settings(&mut tx, &row).await?;
    tx.commit().await?;

    let settings = to_settings(updated);
    runtime::set(settings.clone());
    tracing::info!(
        request_log_retention_days = settings.request_log_retention_days,
        audit_retention_days = settings.audit_retention_days,
        upstream_timeout_secs = settings.upstream_timeout_secs,
        default_provider_id = ?settings.default_provider_id,
        "app_settings updated"
    );
    Ok(settings)
}

/// 范围校验（与 DB CHECK 一致；返回 400 validation_failed 首个错误）。
fn validate_patch(patch: &PatchSettingsRequest) -> Result<(), AppError> {
    if patch.is_empty() {
        return Err(AppError::validation_field(
            "settings",
            "至少提供一个可修改字段",
        ));
    }
    if let Some(value) = patch.request_log_retention_days
        && !(MIN_REQUEST_LOG_RETENTION_DAYS..=MAX_REQUEST_LOG_RETENTION_DAYS).contains(&value)
    {
        return Err(AppError::validation_field(
            "request_log_retention_days",
            format!(
                "request_log_retention_days 必须在 {MIN_REQUEST_LOG_RETENTION_DAYS}–{MAX_REQUEST_LOG_RETENTION_DAYS} 之间"
            ),
        ));
    }
    if let Some(value) = patch.audit_retention_days
        && !(MIN_AUDIT_RETENTION_DAYS..=MAX_AUDIT_RETENTION_DAYS).contains(&value)
    {
        return Err(AppError::validation_field(
            "audit_retention_days",
            format!(
                "audit_retention_days 必须在 {MIN_AUDIT_RETENTION_DAYS}–{MAX_AUDIT_RETENTION_DAYS} 之间"
            ),
        ));
    }
    if let Some(value) = patch.upstream_timeout_secs
        && !(MIN_UPSTREAM_TIMEOUT_SECS..=MAX_UPSTREAM_TIMEOUT_SECS).contains(&value)
    {
        return Err(AppError::validation_field(
            "upstream_timeout_secs",
            format!(
                "upstream_timeout_secs 必须在 {MIN_UPSTREAM_TIMEOUT_SECS}–{MAX_UPSTREAM_TIMEOUT_SECS} 之间"
            ),
        ));
    }
    Ok(())
}

fn to_settings(row: repo::SettingsRow) -> Settings {
    Settings {
        default_provider_id: row.default_provider_id,
        request_log_retention_days: row.request_log_retention_days,
        audit_retention_days: row.audit_retention_days,
        upstream_timeout_secs: row.upstream_timeout_secs,
        updated_at: row.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        extract::ConnectInfo,
        http::{HeaderMap, Method, Request, StatusCode, header},
    };
    use chrono::Utc;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::modules::identity::model::SESSION_COOKIE;
    use crate::modules::identity::security::sha256_hex;
    use crate::platform::config::Config;
    use crate::platform::crypto::Crypto;
    use crate::platform::state::AppState;

    use super::super::runtime;

    fn patch_with(
        request: Option<i32>,
        audit: Option<i32>,
        timeout: Option<i32>,
    ) -> PatchSettingsRequest {
        PatchSettingsRequest {
            default_provider_id: None,
            request_log_retention_days: request,
            audit_retention_days: audit,
            upstream_timeout_secs: timeout,
        }
    }

    #[test]
    fn validate_rejects_empty_and_out_of_range() {
        assert!(matches!(
            validate_patch(&PatchSettingsRequest::default()),
            Err(AppError::Validation { .. })
        ));
        // 请求日志保留 1–365。
        assert!(validate_patch(&patch_with(Some(1), None, None)).is_ok());
        assert!(validate_patch(&patch_with(Some(365), None, None)).is_ok());
        assert!(validate_patch(&patch_with(Some(0), None, None)).is_err());
        assert!(validate_patch(&patch_with(Some(366), None, None)).is_err());
        // 审计保留 180–3650。
        assert!(validate_patch(&patch_with(None, Some(180), None)).is_ok());
        assert!(validate_patch(&patch_with(None, Some(3_650), None)).is_ok());
        assert!(validate_patch(&patch_with(None, Some(179), None)).is_err());
        assert!(validate_patch(&patch_with(None, Some(3_651), None)).is_err());
        // 上游超时 5–600。
        assert!(validate_patch(&patch_with(None, None, Some(5))).is_ok());
        assert!(validate_patch(&patch_with(None, None, Some(600))).is_ok());
        assert!(validate_patch(&patch_with(None, None, Some(4))).is_err());
        assert!(validate_patch(&patch_with(None, None, Some(601))).is_err());
        // 仅 default_provider_id（null 清除）也算至少一个字段。
        let clear_only = PatchSettingsRequest {
            default_provider_id: Some(None),
            ..Default::default()
        };
        assert!(validate_patch(&clear_only).is_ok());
    }

    // ---------- S5.3 gated：迁移/GET/PATCH/runtime/消费方 ----------

    fn test_state(db: Option<sqlx::PgPool>) -> AppState {
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
            db,
            Arc::new(Crypto::from_key_bytes([17u8; 32])),
        )
    }

    async fn test_state_db() -> Option<AppState> {
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
        Some(test_state(Some(pool)))
    }

    async fn http_call(
        app: &Router,
        method: Method,
        uri: &str,
        headers: &[(&str, String)],
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        for (name, value) in headers {
            builder = builder.header(*name, value);
        }
        let mut request = builder.body(Body::empty()).expect("build request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
        if let Some(body) = body {
            *request.body_mut() = Body::from(body.to_string());
        }
        let response = app.clone().oneshot(request).await.expect("request");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }

    async fn reset_app_settings(pool: &sqlx::PgPool) {
        // 其他 gated 用例 `TRUNCATE providers CASCADE` 会级联清空 app_settings（FK 依赖），
        // 测试库需兜底重建单例行；生产环境不做 TRUNCATE。
        sqlx::query("INSERT INTO app_settings (id) VALUES (1) ON CONFLICT (id) DO NOTHING")
            .execute(pool)
            .await
            .expect("ensure app_settings singleton");
        sqlx::query(
            "UPDATE app_settings SET default_provider_id = NULL, \
                 request_log_retention_days = 30, audit_retention_days = 180, \
                 upstream_timeout_secs = 60, updated_at = now() \
             WHERE id = 1",
        )
        .execute(pool)
        .await
        .expect("reset app_settings");
    }

    /// 直接插入管理员 + Session（返回 cookie 值、CSRF token）。
    async fn seed_admin_session(state: &AppState) -> (Uuid, String, String) {
        let pool = state.db.as_ref().expect("pool");
        let admin_id = Uuid::new_v4();
        let email = format!("settings-s53-{admin_id}@corp.example.com");
        sqlx::query(
            "INSERT INTO admin_users (id, email, password_hash) VALUES ($1, $2, 'not-a-real-hash')",
        )
        .bind(admin_id)
        .bind(&email)
        .execute(pool)
        .await
        .expect("insert admin");
        let token = format!("s53-token-{admin_id}");
        let token_hash = sha256_hex(&token);
        sqlx::query(
            "INSERT INTO admin_sessions (id, user_id, csrf_token, token_hash, expires_at) \
             VALUES ($1, $2, 'csrf-token', $3, now() + interval '1 hour')",
        )
        .bind(Uuid::new_v4())
        .bind(admin_id)
        .bind(token_hash)
        .execute(pool)
        .await
        .expect("insert session");
        (
            admin_id,
            format!("{SESSION_COOKIE}={token}"),
            "csrf-token".to_string(),
        )
    }

    async fn seed_provider(pool: &sqlx::PgPool) -> Uuid {
        let provider_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO providers (id, name, kind, base_url) \
             VALUES ($1, $2, 'openai_compatible', 'http://127.0.0.1:1/v1')",
        )
        .bind(provider_id)
        .bind(format!("settings-s53-{provider_id}"))
        .execute(pool)
        .await
        .expect("insert provider");
        provider_id
    }

    async fn latest_settings_row(pool: &sqlx::PgPool) -> (Option<Uuid>, i32, i32, i32) {
        sqlx::query_as(
            "SELECT default_provider_id, request_log_retention_days, \
                    audit_retention_days, upstream_timeout_secs \
             FROM app_settings WHERE id = 1",
        )
        .fetch_one(pool)
        .await
        .expect("settings row")
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（HTTP settings + runtime + cleanup；用例随机 id 并重置设置）"]
    async fn settings_persist_runtime_and_consumers() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let app = crate::router::app(state.clone());
        let pool = state.db.as_ref().expect("pool").clone();
        let started = Utc::now();

        // 基线：DB 默认值 + runtime 加载（进程内全局，先重置避免其他用例影响）。
        reset_app_settings(&pool).await;
        initialize(&state).await;
        let current = runtime::current();
        assert_eq!(current.default_provider_id, None);
        assert_eq!(current.request_log_retention_days, 30);
        assert_eq!(current.audit_retention_days, 180);
        assert_eq!(current.upstream_timeout_secs, 60);
        let row_count: i64 = sqlx::query_scalar("SELECT count(*) FROM app_settings")
            .fetch_one(&pool)
            .await
            .expect("settings row count");
        assert_eq!(row_count, 1, "app_settings must be a singleton");

        let (admin_id, cookie, csrf) = seed_admin_session(&state).await;
        let auth = [("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())];

        // 未登录 401；登录 GET 200 + 默认值 + no-store。
        let (status, _, _) = http_call(&app, Method::GET, "/api/admin/settings", &[], None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, headers, body) = http_call(
            &app,
            Method::GET,
            "/api/admin/settings",
            &[("cookie", cookie.clone())],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={body}");
        assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
        let defaults: Value = serde_json::from_str(&body).expect("settings json");
        assert_eq!(defaults["default_provider_id"], Value::Null);
        assert_eq!(defaults["request_log_retention_days"], 30);
        assert_eq!(defaults["audit_retention_days"], 180);
        assert_eq!(defaults["upstream_timeout_secs"], 60);
        assert!(defaults["updated_at"].as_str().is_some());

        // PATCH 全字段：持久化 + runtime + proxy 超时生效。
        let provider_id = seed_provider(&pool).await;
        let patch_payload = json!({
            "default_provider_id": provider_id,
            "request_log_retention_days": 7,
            "audit_retention_days": 200,
            "upstream_timeout_secs": 30,
        });
        let (status, headers, body) = http_call(
            &app,
            Method::PATCH,
            "/api/admin/settings",
            &auth,
            Some(patch_payload),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={body}");
        assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
        let patched: Value = serde_json::from_str(&body).expect("patched json");
        assert_eq!(patched["default_provider_id"], provider_id.to_string());
        assert_eq!(patched["request_log_retention_days"], 7);
        assert_eq!(patched["audit_retention_days"], 200);
        assert_eq!(patched["upstream_timeout_secs"], 30);
        assert_eq!(
            latest_settings_row(&pool).await,
            (Some(provider_id), 7, 200, 30)
        );
        let current = runtime::current();
        assert_eq!(current.default_provider_id, Some(provider_id));
        assert_eq!(current.request_log_retention_days, 7);
        assert_eq!(current.audit_retention_days, 200);
        assert_eq!(current.upstream_timeout_secs, 30);
        // proxy 读取 runtime：headers/stream-idle 超时随 upstream_timeout_secs 变化，其余不变。
        let proxy = crate::modules::proxy::settings::ProxySettings::from_config(&state.config);
        assert_eq!(proxy.headers_timeout, Duration::from_secs(30));
        assert_eq!(proxy.stream_idle_timeout, Duration::from_secs(30));
        assert_eq!(proxy.non_stream_total_timeout, Duration::from_secs(300));
        assert_eq!(proxy.stream_max_duration, Duration::from_secs(900));

        // 审计 middleware：settings 写 → platform.settings.updated / app_settings。
        assert!(crate::modules::observability::service::flush(Duration::from_secs(5)).await);
        let audit_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_logs \
             WHERE created_at >= $1 AND action = 'platform.settings.updated' AND outcome = 'success'",
        )
        .bind(started)
        .fetch_one(&pool)
        .await
        .expect("settings audit count");
        assert!(audit_count >= 1);
        let (resource_type, actor_id): (String, Option<Uuid>) = sqlx::query_as(
            "SELECT resource_type, actor_id FROM audit_logs \
             WHERE created_at >= $1 AND action = 'platform.settings.updated' AND outcome = 'success' \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(started)
        .fetch_one(&pool)
        .await
        .expect("settings audit row");
        assert_eq!(resource_type, "app_settings");
        assert_eq!(actor_id, Some(admin_id));

        // 清理任务读 runtime：保留 7 天 → 10 天前请求日志删除、2 天前保留。
        let keep_request = format!("s53-keep-{started}");
        let old_request = format!("s53-old-{started}");
        sqlx::query(
            "INSERT INTO request_logs (id, request_id, method, path, created_at) VALUES \
                 (gen_random_uuid(), $1, 'GET', '/v1/models', $2), \
                 (gen_random_uuid(), $3, 'GET', '/v1/models', $4)",
        )
        .bind(&keep_request)
        .bind(Utc::now() - chrono::Duration::days(2))
        .bind(&old_request)
        .bind(Utc::now() - chrono::Duration::days(10))
        .execute(&pool)
        .await
        .expect("seed request logs");
        // 审计保留 200 天 → 250 天前删除、10 天前保留。
        let keep_audit = "s53.cleanup.recent".to_string();
        let old_audit = "s53.cleanup.old".to_string();
        sqlx::query(
            "INSERT INTO audit_logs (id, created_at, actor_type, action, resource_type, outcome) VALUES \
                 (gen_random_uuid(), $1, 'system', $2, 'system', 'success'), \
                 (gen_random_uuid(), $3, 'system', $4, 'system', 'success')",
        )
        .bind(Utc::now() - chrono::Duration::days(10))
        .bind(&keep_audit)
        .bind(Utc::now() - chrono::Duration::days(250))
        .bind(&old_audit)
        .execute(&pool)
        .await
        .expect("seed audit logs");
        crate::modules::observability::service::run_cleanup_once(&state).await;
        let old_request_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM request_logs WHERE request_id = $1")
                .bind(&old_request)
                .fetch_one(&pool)
                .await
                .expect("count old request log");
        assert_eq!(
            old_request_left, 0,
            "runtime retention=7 must delete 10d old"
        );
        let keep_request_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM request_logs WHERE request_id = $1")
                .bind(&keep_request)
                .fetch_one(&pool)
                .await
                .expect("count kept request log");
        assert_eq!(keep_request_left, 1, "2d old must be kept");
        let kept_audit: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = $1")
                .bind(&keep_audit)
                .fetch_one(&pool)
                .await
                .expect("count kept audit");
        assert_eq!(kept_audit, 1);
        let old_audit_left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = $1")
                .bind(&old_audit)
                .fetch_one(&pool)
                .await
                .expect("count old audit");
        assert_eq!(
            old_audit_left, 0,
            "runtime audit retention=200 must delete 250d old"
        );

        // 校验：空 PATCH/范围/未知字段/CSRF/provider 不存在。
        let (status, _, body) = http_call(
            &app,
            Method::PATCH,
            "/api/admin/settings",
            &auth,
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body_code(&body), "validation_failed");
        for payload in [
            json!({"request_log_retention_days": 0}),
            json!({"request_log_retention_days": 366}),
            json!({"audit_retention_days": 179}),
            json!({"audit_retention_days": 3651}),
            json!({"upstream_timeout_secs": 4}),
            json!({"upstream_timeout_secs": 601}),
            json!({"bogus": true}),
        ] {
            let (status, _, body) = http_call(
                &app,
                Method::PATCH,
                "/api/admin/settings",
                &auth,
                Some(payload),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        }
        let missing_provider = Uuid::new_v4();
        let (status, _, body) = http_call(
            &app,
            Method::PATCH,
            "/api/admin/settings",
            &auth,
            Some(json!({"default_provider_id": missing_provider})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
        assert_eq!(body_code(&body), "provider_not_found");
        // 无 CSRF → 403，且不改变设置。
        let (status, _, _) = http_call(
            &app,
            Method::PATCH,
            "/api/admin/settings",
            &[("cookie", cookie.clone())],
            Some(json!({"request_log_retention_days": 5})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(runtime::current().request_log_retention_days, 7);
        // null 清除默认供应商。
        let (status, _, body) = http_call(
            &app,
            Method::PATCH,
            "/api/admin/settings",
            &auth,
            Some(json!({"default_provider_id": null})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={body}");
        assert_eq!(runtime::current().default_provider_id, None);

        // runtime 重载：直接改 DB → initialize 重新加载。
        sqlx::query(
            "UPDATE app_settings SET request_log_retention_days = 45, updated_at = now() WHERE id = 1",
        )
        .execute(&pool)
        .await
        .expect("update settings for reload");
        initialize(&state).await;
        assert_eq!(runtime::current().request_log_retention_days, 45);

        // DB 不可用：initialize 回退 env/默认（30/180/60）并 warn。
        let offline = test_state(None);
        initialize(&offline).await;
        let fallback = runtime::current();
        assert_eq!(fallback.request_log_retention_days, 30);
        assert_eq!(fallback.audit_retention_days, 180);
        assert_eq!(fallback.upstream_timeout_secs, 60);

        // 恢复默认并清理测试数据（保证后续 gated 用例看到 30/180/60）。
        reset_app_settings(&pool).await;
        initialize(&state).await;
        let _ = sqlx::query("DELETE FROM request_logs WHERE request_id = ANY($1)")
            .bind(vec![keep_request, old_request])
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM audit_logs WHERE created_at >= $1")
            .bind(started)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM admin_sessions WHERE user_id = $1")
            .bind(admin_id)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM admin_users WHERE id = $1")
            .bind(admin_id)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM providers WHERE id = $1")
            .bind(provider_id)
            .execute(&pool)
            .await;
    }

    fn body_code(body: &str) -> String {
        serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| value["code"].as_str().map(str::to_string))
            .unwrap_or_default()
    }
}
