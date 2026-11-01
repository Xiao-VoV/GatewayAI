//! HTTP 层契约测试（需要 Postgres，`GW_TEST_DATABASE_URL` gated）。
//!
//! 覆盖 identity + providers + access 的状态码、problem+json、响应头（x-request-id/no-store）、
//! Cookie 属性、CSRF、敏感字段与错误码，作为 handler 与 `接口契约.yaml` 的对拍。
//!
//! 运行：
//! `GW_TEST_DATABASE_URL=postgres://gw:gw@127.0.0.1:5432/gw_test \
//!  cargo test --all-targets -- --ignored --test-threads=1`

#![cfg(test)]

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use crate::platform::{config::Config, crypto::Crypto, state::AppState};
use std::sync::Arc;

async fn test_app() -> Option<(Router, PgPool)> {
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
    for table in ["admin_users", "providers"] {
        sqlx::query(&format!("TRUNCATE {table} CASCADE"))
            .execute(&pool)
            .await
            .expect("truncate test tables");
    }
    let config = Config {
        bind: "127.0.0.1:0".parse().expect("bind addr"),
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
    let state = AppState {
        config: Arc::new(config),
        db: Some(pool.clone()),
        crypto: Arc::new(Crypto::from_key_bytes([3u8; 32])),
        http: crate::platform::http::shared_client(),
        started_at: std::time::Instant::now(),
    };
    Some((crate::router::app(state), pool))
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, value);
    }
    let mut request = builder
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("build request");
    // identity login/setup handler 使用 ConnectInfo（真实运行时由 serve 注入）。
    request
        .extensions_mut()
        .insert(ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            12345,
        ))));
    let response = app.clone().oneshot(request).await.expect("request");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, headers, value)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn cookie_pair(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn assert_problem(headers: &HeaderMap, body: &Value, status: StatusCode, code: &str) {
    assert_eq!(
        header_str(headers, "content-type"),
        Some("application/problem+json")
    );
    assert_eq!(body["status"].as_u64(), Some(status.as_u16() as u64));
    assert_eq!(body["code"].as_str(), Some(code));
    assert!(body["detail"].as_str().is_some());
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE admin_users/providers"]
async fn identity_and_providers_http_contract() {
    let Some((app, _pool)) = test_app().await else {
        return;
    };

    // ---- identity setup ----
    let (status, headers, body) = call(&app, Method::GET, "/api/admin/auth/setup", &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["required"].as_bool(), Some(true));
    assert_eq!(body["setup_token_required"].as_bool(), Some(false));
    assert!(header_str(&headers, "x-request-id").is_some());
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));

    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "Admin@Corp.Example.com",
            "password": "change-me-please-123",
            "display_name": " 平台管理员 "
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["email"].as_str(), Some("admin@corp.example.com"));
    assert_eq!(body["display_name"].as_str(), Some("平台管理员"));
    assert!(body.get("password").is_none());
    assert!(body.get("password_hash").is_none());
    assert!(header_str(&headers, "cache-control") == Some("no-store"));

    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({"email": "other@corp.example.com", "password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"].as_str(), Some("setup_already_completed"));

    // ---- login ----
    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({"email": "admin@corp.example.com", "password": "wrong-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_problem(
        &headers,
        &body,
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
    );
    assert!(headers.get(header::SET_COOKIE).is_none());

    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({"email": "admin@corp.example.com", "password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let set_cookie = header_str(&headers, "set-cookie").expect("session cookie");
    for expected in ["HttpOnly", "SameSite=Lax", "Max-Age=43200"] {
        assert!(
            set_cookie.contains(expected),
            "cookie 缺少 {expected}: {set_cookie}"
        );
    }
    let cookie = cookie_pair(set_cookie);
    let csrf = body["csrf_token"].as_str().expect("csrf").to_string();
    assert_eq!(csrf.len(), 43, "CSRF 应为 32 字节 base64url（43 字符）");

    // ---- me / logout ----
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/auth/me",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["user"]["email"].as_str(),
        Some("admin@corp.example.com")
    );

    let (status, headers, body) = call(&app, Method::GET, "/api/admin/auth/me", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_problem(
        &headers,
        &body,
        StatusCode::UNAUTHORIZED,
        "not_authenticated",
    );

    // ---- providers：未登录/缺 CSRF/校验 ----
    let (status, _, body) = call(&app, Method::GET, "/api/admin/providers", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"].as_str(), Some("not_authenticated"));

    let provider_body = json!({
        "name": "HttpContractProvider",
        "kind": "openai_compatible",
        "base_url": "http://127.0.0.1:1/v1",
        "description": "contract",
        "account": {"name": "主账号", "api_key": "sk-secret-1234"}
    });
    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/providers",
        &[("cookie", cookie.clone())],
        Some(provider_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_problem(&headers, &body, StatusCode::FORBIDDEN, "csrf_invalid");

    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/providers",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({
            "name": "bad",
            "kind": "openai_compatible",
            "base_url": "https://api.example.com",
            "account": {"name": "a", "api_key": "sk-x"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"].as_str(), Some("validation_failed"));
    assert!(body["errors"]["base_url"].as_str().is_some());

    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/providers",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({
            "name": "anth",
            "kind": "anthropic",
            "base_url": "https://api.anthropic.com/v1",
            "account": {"name": "a", "api_key": "sk-x"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"].as_str(), Some("unsupported_kind"));

    // ---- providers：创建/列表/详情/测试 ----
    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/providers",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(provider_body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["account"]["api_key_hint"].as_str(), Some("sk-…1234"));
    assert!(body["account"].get("api_key").is_none());
    assert!(body.to_string().find("sk-secret-1234").is_none());
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    let provider_id = body["provider"]["id"]
        .as_str()
        .expect("provider id")
        .to_string();

    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/providers",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64(), Some(1));

    let (status, _, body) = call(
        &app,
        Method::GET,
        &format!("/api/admin/providers/{provider_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["account_count"].as_u64(), Some(1));
    assert_eq!(body["routable"].as_bool(), Some(true));

    let (status, headers, body) = call(
        &app,
        Method::POST,
        &format!("/api/admin/providers/{provider_id}/test"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"model": "deepseek-chat"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(false));
    let error_code = body["error"]["code"].as_str().expect("error code");
    assert!(
        matches!(error_code, "connect_error" | "timeout" | "dns_error"),
        "unexpected probe error: {error_code}"
    );
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));

    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/providers/00000000-0000-0000-0000-000000000000",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"].as_str(), Some("provider_not_found"));

    // ---- logout ----
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/logout",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"].as_str(), Some("csrf_invalid"));

    let (status, headers, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/logout",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let cleared = header_str(&headers, "set-cookie").expect("clear cookie");
    assert!(cleared.contains("Max-Age=0"), "clear cookie: {cleared}");

    let (status, _, _) = call(
        &app,
        Method::GET,
        "/api/admin/auth/me",
        &[("cookie", cookie)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE access 表"]
async fn access_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query("TRUNCATE users, devices, pairing_codes, device_tokens, virtual_keys CASCADE")
        .execute(&pool)
        .await
        .expect("truncate access tables");

    // 管理台初始化/登录
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "access-http-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({
            "email": "access-http-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = body["csrf_token"].as_str().expect("csrf").to_string();

    let (status, _, _) = call(&app, Method::GET, "/api/admin/users", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/pairing-codes",
        &[("cookie", cookie.clone())],
        Some(json!({"user_email": "alice@corp.example.com"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"].as_str(), Some("csrf_invalid"));

    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/pairing-codes",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({
            "user_email": "alice@corp.example.com",
            "user_display_name": "Alice"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    let code = body["pairing_code"]["code"]
        .as_str()
        .expect("pairing code")
        .to_string();
    assert!(body["pairing_code"]["user"]["email"].as_str().is_some());

    let (status, headers, paired) = call(
        &app,
        Method::POST,
        "/api/client/pair",
        &[],
        Some(json!({
            "code": code,
            "device": {"name": "Alice Mac", "platform": "macos", "client_version": "0.1.0"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    let api_key = paired["api_key"].as_str().expect("api key").to_string();
    let device_secret = paired["device_secret"]
        .as_str()
        .expect("device secret")
        .to_string();
    assert!(paired["token"].as_str().is_some());
    let device_id = paired["device"]["id"]
        .as_str()
        .expect("device id")
        .to_string();
    let key_prefix = paired["api_key_prefix"]
        .as_str()
        .expect("key prefix")
        .to_string();
    assert_eq!(key_prefix, &api_key[..12]);

    // token 交换 + 客户端设备信息
    let (status, _, token_body) = call(
        &app,
        Method::POST,
        "/api/client/token",
        &[],
        Some(json!({"device_id": device_id, "device_secret": device_secret})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = token_body["token"].as_str().expect("token").to_string();

    let (status, headers, device_body) = call(
        &app,
        Method::GET,
        "/api/client/device",
        &[("authorization", format!("Bearer {token}"))],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(
        device_body["user"]["email"].as_str(),
        Some("alice@corp.example.com")
    );
    assert_eq!(
        device_body["key"]["key_prefix"].as_str(),
        Some(key_prefix.as_str())
    );
    assert!(device_body.get("api_key").is_none());
    assert!(!device_body.to_string().contains(&api_key));

    // 客户端用量：双因子、本人范围、no-store、days 校验
    let (status, headers, usage) = call(
        &app,
        Method::GET,
        "/api/client/usage?days=7",
        &[
            ("authorization", format!("Bearer {api_key}")),
            ("x-gw-device-token", token.clone()),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(usage["days"].as_i64(), Some(7));
    assert_eq!(usage["currency"].as_str(), Some("CNY"));
    assert_eq!(usage["totals"]["requests"].as_i64(), Some(0));
    assert!(usage.get("provider").is_none());
    assert!(!usage.to_string().contains(&api_key));
    let (status, _, _) = call(
        &app,
        Method::GET,
        "/api/client/usage?days=0",
        &[
            ("authorization", format!("Bearer {api_key}")),
            ("x-gw-device-token", token.clone()),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(&app, Method::GET, "/api/client/usage", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 管理台设备/Key 列表 + 无明文
    let (status, _, devices) = call(
        &app,
        Method::GET,
        "/api/admin/devices",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(devices["total"].as_u64(), Some(1));
    let key_id = devices["items"][0]["key"]["id"]
        .as_str()
        .expect("key id")
        .to_string();
    let (status, _, keys) = call(
        &app,
        Method::GET,
        "/api/admin/keys",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys["total"].as_u64(), Some(1));
    assert!(!keys.to_string().contains(&api_key));
    assert!(!keys.to_string().contains(&device_secret));

    // PATCH 白名单 + 重置（旧 token 失效）+ 吊销幂等
    let (status, _, key_patched) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/keys/{key_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"allowed_models": ["claude-sonnet-4-5"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        key_patched["allowed_models"][0].as_str(),
        Some("claude-sonnet-4-5")
    );
    // RPM 配置：设置/非法/清除
    let (status, _, rpm_patched) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/keys/{key_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"rpm_limit": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rpm_patched["rpm_limit"], 2);
    let (status, _, body) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/keys/{key_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"rpm_limit": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");
    let (status, _, rpm_cleared) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/keys/{key_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"rpm_limit": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rpm_cleared["rpm_limit"], serde_json::Value::Null);

    let (status, _, reset) = call(
        &app,
        Method::POST,
        &format!("/api/admin/keys/{key_id}/reset"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(reset["api_key"].as_str().is_some());
    let (status, _, _) = call(
        &app,
        Method::GET,
        "/api/client/device",
        &[("authorization", format!("Bearer {token}"))],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/keys/{key_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/keys/{key_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 设备禁用后 token 交换 403
    let (status, _, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/devices/{device_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"status": "disabled"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/client/token",
        &[],
        Some(json!({"device_id": device_id, "device_secret": device_secret})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"].as_str(), Some("device_disabled"));

    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/client/pair",
        &[],
        Some(json!({"code": "ZZZZ-ZZZZ"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"].as_str(), Some("pairing_code_invalid"));
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE users/budgets"]
async fn policy_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query("TRUNCATE users CASCADE")
        .execute(&pool)
        .await
        .expect("truncate users/budgets");

    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "policy-http-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({
            "email": "policy-http-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = body["csrf_token"].as_str().expect("csrf").to_string();

    let user_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, display_name) VALUES ($1, $2, 'Policy HTTP')")
        .bind(user_id)
        .bind(format!("{user_id}@corp.example.com"))
        .execute(&pool)
        .await
        .expect("insert user");
    let user_id = user_id.to_string();

    let (status, _, _) = call(&app, Method::GET, "/api/admin/budgets", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, body) = call(
        &app,
        Method::PUT,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone())],
        Some(json!({"daily_limit": "5"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"].as_str(), Some("csrf_invalid"));

    let (status, _, body) = call(
        &app,
        Method::PUT,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"daily_limit": "-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"].as_str(), Some("validation_failed"));

    let (status, _, body) = call(
        &app,
        Method::PUT,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"daily_limit": "5", "currency": "USD"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"].as_str(), Some("unsupported_currency"));

    let (status, headers, budget) = call(
        &app,
        Method::PUT,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"daily_limit": "5", "currency": "CNY"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(budget["daily_limit"].as_str(), Some("5.000000"));
    assert_eq!(budget["currency"].as_str(), Some("CNY"));

    let (status, _, list) = call(
        &app,
        Method::GET,
        "/api/admin/budgets",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["total"].as_u64(), Some(1));

    let (status, _, fetched) = call(
        &app,
        Method::GET,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["user_id"].as_str(), Some(user_id.as_str()));

    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/budgets/00000000-0000-0000-0000-000000000000",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"].as_str(), Some("budget_not_found"));

    let (status, _, body) = call(
        &app,
        Method::PUT,
        "/api/admin/budgets/00000000-0000-0000-0000-000000000000",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({"daily_limit": "1"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"].as_str(), Some("user_not_found"));

    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, headers, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));

    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(
        &app,
        Method::GET,
        &format!("/api/admin/budgets/{user_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"].as_str(), Some("budget_not_found"));
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE providers/model_aliases"]
async fn routing_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query("TRUNCATE providers, model_aliases CASCADE")
        .execute(&pool)
        .await
        .expect("truncate routing tables");

    let provider_a = uuid::Uuid::new_v4();
    let provider_b = uuid::Uuid::new_v4();
    let model_a = uuid::Uuid::new_v4();
    let model_b = uuid::Uuid::new_v4();
    for (id, name) in [(provider_a, "RouteA"), (provider_b, "RouteB")] {
        sqlx::query(
            "INSERT INTO providers (id, name, kind, base_url) \
             VALUES ($1, $2, 'openai_compatible', 'http://localhost/v1')",
        )
        .bind(id)
        .bind(name)
        .execute(&pool)
        .await
        .expect("insert provider");
    }
    for (id, provider, name) in [
        (model_a, provider_a, "route-a"),
        (model_b, provider_b, "route-b"),
    ] {
        sqlx::query(
            "INSERT INTO upstream_models (id, provider_id, name, canonical, capabilities) \
             VALUES ($1, $2, $3, $3, '[\"chat\"]'::jsonb)",
        )
        .bind(id)
        .bind(provider)
        .bind(name)
        .execute(&pool)
        .await
        .expect("insert upstream model");
    }

    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "routing-http-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({
            "email": "routing-http-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = body["csrf_token"].as_str().expect("csrf").to_string();

    let (status, _, _) = call(&app, Method::GET, "/api/admin/model-aliases", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let create_body = json!({
        "alias": "http-fast",
        "enabled": true,
        "targets": [
            {"upstream_model_id": model_a, "priority": 1, "weight": 80, "enabled": true},
            {"upstream_model_id": model_b, "priority": 2, "weight": 20, "enabled": true}
        ]
    });
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/model-aliases",
        &[("cookie", cookie.clone())],
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"].as_str(), Some("csrf_invalid"));

    let (status, headers, alias) = call(
        &app,
        Method::POST,
        "/api/admin/model-aliases",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(alias["alias"].as_str(), Some("http-fast"));
    assert_eq!(alias["targets"].as_array().map(Vec::len), Some(2));
    let alias_id = alias["id"].as_str().expect("alias id").to_string();

    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/model-aliases",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(create_body),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"].as_str(), Some("model_alias_conflict"));

    let missing = json!({
        "alias": "missing-target",
        "targets": [{"upstream_model_id": uuid::Uuid::new_v4()}]
    });
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/model-aliases",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(missing),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"].as_str(), Some("upstream_model_not_found"));

    let (status, _, list) = call(
        &app,
        Method::GET,
        "/api/admin/model-aliases?limit=10&offset=0",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["total"].as_u64(), Some(1));

    let (status, _, fetched) = call(
        &app,
        Method::GET,
        &format!("/api/admin/model-aliases/{alias_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["alias"].as_str(), Some("http-fast"));

    let (status, _, preview) = call(
        &app,
        Method::GET,
        &format!("/api/admin/model-aliases/{alias_id}/preview"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["candidates"].as_array().map(Vec::len), Some(2));
    assert_eq!(preview["candidates"][0]["priority"].as_i64(), Some(1));

    let patch = json!({
        "alias": "http-fast-v2",
        "enabled": false,
        "targets": [{"upstream_model_id": model_b, "priority": 1, "weight": 100}]
    });
    let (status, _, updated) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/model-aliases/{alias_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(patch),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["alias"].as_str(), Some("http-fast-v2"));
    assert_eq!(updated["enabled"].as_bool(), Some(false));
    assert_eq!(updated["targets"].as_array().map(Vec::len), Some(1));

    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/model-aliases/00000000-0000-0000-0000-000000000000",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"].as_str(), Some("model_alias_not_found"));

    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/model-aliases/{alias_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/model-aliases/{alias_id}"),
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call(
        &app,
        Method::GET,
        &format!("/api/admin/model-aliases/{alias_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE 测试表"]
async fn catalog_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query(
        "TRUNCATE admin_users, users, providers, model_aliases, model_groups, request_logs, usage_records CASCADE",
    )
    .execute(&pool)
    .await
    .expect("truncate catalog test tables");

    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "catalog-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, login) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({
            "email": "catalog-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = login["csrf_token"].as_str().expect("csrf").to_string();

    // pairing（仅需一个已鉴权 subject）
    let (status, _, pair_code) = call(
        &app,
        Method::POST,
        "/api/admin/pairing-codes",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({
            "user_email": "catalog-user@corp.example.com",
            "user_display_name": "Catalog User",
            "expires_in_seconds": 600
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let code = pair_code["pairing_code"]["code"].as_str().expect("code");
    let (status, _, paired) = call(
        &app,
        Method::POST,
        "/api/client/pair",
        &[],
        Some(json!({
            "code": code,
            "device": {"name": "Catalog Mac", "platform": "macos", "client_version": "0.1.0"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let auth = [
        (
            "authorization",
            format!("Bearer {}", paired["api_key"].as_str().expect("key")),
        ),
        (
            "x-gw-device-token",
            paired["token"].as_str().expect("token").to_string(),
        ),
    ];

    // 无发布 → 空目录 200 + cat:<n>；不回退 S4 alias
    let (status, headers, list) = call(&app, Method::GET, "/v1/models", &auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    let version = header_str(&headers, "x-gw-catalog-version").expect("version");
    assert!(version.starts_with("cat:"), "{version}");
    assert_eq!(list["object"], "list");
    assert_eq!(list["catalog_version"], version);
    assert_eq!(list["data"], json!([]));

    let (status, _, body) = call(&app, Method::GET, "/v1/models/none", &auth, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "model_not_found");

    let flush = std::time::Duration::from_secs(2);
    crate::modules::observability::service::flush(flush).await;
    let log_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM request_logs WHERE protocol = 'models'")
            .fetch_one(&pool)
            .await
            .expect("log count");
    assert!(log_count >= 2, "expected models logs, got {log_count}");
    let usage_count: i64 = sqlx::query_scalar("SELECT count(*) FROM usage_records")
        .fetch_one(&pool)
        .await
        .expect("usage count");
    assert_eq!(usage_count, 0);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE usage_records"]
async fn metering_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query("TRUNCATE usage_records CASCADE")
        .execute(&pool)
        .await
        .expect("truncate usage");
    let now = chrono::Utc::now();
    for (request_id, alias, status, input, output, cost, offset_secs) in [
        ("mtr-1", "fast", "success", 100, 40, "0.100000", 60_i64),
        ("mtr-2", "smart", "upstream_error", 50, 0, "0.000000", 120),
        ("mtr-3", "fast", "rejected", 10, 0, "0.000000", 25 * 3600),
    ] {
        sqlx::query(
            "INSERT INTO usage_records \
             (id, request_id, model_alias, input_tokens, output_tokens, cost, currency, latency_ms, status, created_at, source, usage_source) \
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5::text::numeric, 'CNY', 10, $6, $7, 'alias', 'upstream')",
        )
        .bind(request_id)
        .bind(alias)
        .bind(input)
        .bind(output)
        .bind(cost)
        .bind(status)
        .bind(now - chrono::Duration::seconds(offset_secs))
        .execute(&pool)
        .await
        .expect("seed usage");
    }

    let (status, _, _) = call(&app, Method::GET, "/api/admin/overview", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "metering-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, _body) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({
            "email": "metering-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));

    let (status, headers, overview) = call(
        &app,
        Method::GET,
        "/api/admin/overview?days=7",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(overview["currency"], "CNY");
    assert_eq!(overview["timezone"], "Asia/Shanghai");
    assert_eq!(overview["days"], 7);
    assert_eq!(overview["trend"].as_array().map(Vec::len), Some(7));
    assert_eq!(overview["totals"]["requests"], 3);
    assert_eq!(overview["today"]["requests"], 2);
    let breakdown = &overview["today"]["status_breakdown"];
    assert_eq!(breakdown["success"], 1);
    assert_eq!(breakdown["upstream_error"], 1);
    assert_eq!(breakdown["rejected"], 0);
    assert_eq!(overview["totals"]["cost"], "0.100000");
    assert!(
        overview["today"]["error_rate"]
            .as_f64()
            .is_some_and(|v| v > 0.0)
    );

    let from = (now - chrono::Duration::days(3)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to = (now + chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let uri =
        format!("/api/admin/usage?from={from}&to={to}&group_by=day,model_alias&limit=50&offset=0");
    let (status, _, usage) =
        call(&app, Method::GET, &uri, &[("cookie", cookie.clone())], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(usage["group_by"], json!(["day", "model_alias"]));
    assert_eq!(usage["totals"]["requests"], 3);
    assert_eq!(usage["total"], 3);
    assert_eq!(usage["items"].as_array().map(Vec::len), Some(3));
    for item in usage["items"].as_array().expect("items") {
        assert!(item.get("day").is_some());
        assert!(item.get("model_alias").is_some());
        assert!(item.get("key_id").is_none());
        assert!(item["cost"].as_str().is_some_and(|v| v.len() == 8));
    }

    // 空区间
    let empty_from =
        (now + chrono::Duration::days(10)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let empty_to =
        (now + chrono::Duration::days(11)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let uri = format!("/api/admin/usage?from={empty_from}&to={empty_to}");
    let (status, _, empty) =
        call(&app, Method::GET, &uri, &[("cookie", cookie.clone())], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["items"], json!([]));
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["totals"]["requests"], 0);
    assert_eq!(empty["totals"]["cost"], "0.000000");

    // 参数校验
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/overview?days=31",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/usage?group_by=bogus",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_group_by");
    let (status, _, body) = call(
        &app,
        Method::GET,
        &format!("/api/admin/usage?from={to}&to={from}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_time_range");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE request_logs/usage_records"]
async fn observability_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query("TRUNCATE request_logs, usage_records CASCADE")
        .execute(&pool)
        .await
        .expect("truncate observability");
    let now = chrono::Utc::now();
    let rows = [
        (
            "obs-1",
            "chat_completions",
            200,
            Some("fast"),
            Some("sk-gw-abcd"),
            Some("DeepSeek"),
            1,
        ),
        (
            "obs-2",
            "chat_completions",
            429,
            Some("fast"),
            Some("sk-gw-abcd"),
            Some("DeepSeek"),
            2,
        ),
        ("obs-3", "models", 200, None, None, None, 3),
        ("obs-4", "chat_completions", 401, None, None, None, 4),
    ];
    for (request_id, protocol, status, model, prefix, provider, offset_hours) in rows {
        sqlx::query(
            "INSERT INTO request_logs \
             (id, request_id, trace_id, method, path, protocol, user_id, key_prefix, model_alias, provider, is_stream, fallback, attempt_count, status_code, latency_ms, ttft_ms, response_size, created_at) \
             VALUES (gen_random_uuid(), $1, $1, 'POST', $2, $3, NULL, $4, $5, $6, false, false, 1, $7, 10, NULL, 100, $8)",
        )
        .bind(request_id)
        .bind(if protocol == "models" { "/v1/models" } else { "/v1/chat/completions" })
        .bind(protocol)
        .bind(prefix)
        .bind(model)
        .bind(provider)
        .bind(status)
        .bind(now - chrono::Duration::hours(offset_hours))
        .execute(&pool)
        .await
        .expect("seed request log");
    }
    sqlx::query(
        "INSERT INTO usage_records \
         (id, request_id, model_alias, input_tokens, output_tokens, cost, currency, latency_ms, ttft_ms, is_stream, fallback, status, http_status, usage_source, created_at, source) \
         VALUES (gen_random_uuid(), 'obs-1', 'fast', 12, 6, 0.003120, 'CNY', 10, NULL, false, false, 'success', 200, 'upstream', $1, 'alias')",
    )
    .bind(now - chrono::Duration::hours(1))
    .execute(&pool)
    .await
    .expect("seed usage");

    let (status, _, _) = call(&app, Method::GET, "/api/admin/requests", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "obs-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({
            "email": "obs-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let auth = [("cookie", cookie.clone())];

    let (status, headers, page) = call(
        &app,
        Method::GET,
        "/api/admin/requests?limit=50&offset=0",
        &auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(page["total"], 4);
    assert_eq!(page["items"].as_array().map(Vec::len), Some(4));
    let first = &page["items"][0];
    assert_eq!(first["request_id"], "obs-1"); // created_at DESC
    assert!(first.get("trace_id").is_some());
    assert!(first.get("response_size").is_some());

    let (status, _, filtered) = call(
        &app,
        Method::GET,
        "/api/admin/requests?status_class=4xx&limit=50",
        &auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(filtered["total"], 2);
    let (status, _, filtered) = call(
        &app,
        Method::GET,
        "/api/admin/requests?trace_id=obs-1",
        &auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["items"][0]["request_id"], "obs-1");
    let (status, _, filtered) = call(
        &app,
        Method::GET,
        "/api/admin/requests?model=fast&status_code=200",
        &auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(filtered["total"], 1);

    // 详情：usage 合并
    let (status, _, detail) =
        call(&app, Method::GET, "/api/admin/requests/obs-1", &auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["request_id"], "obs-1");
    assert_eq!(detail["usage"]["input_tokens"], 12);
    assert_eq!(detail["usage"]["output_tokens"], 6);
    assert_eq!(detail["usage"]["cost"], "0.003120");
    assert_eq!(detail["usage"]["currency"], "CNY");
    assert_eq!(detail["usage"]["status"], "success");
    // 非推理请求 usage=null
    let (status, _, detail) =
        call(&app, Method::GET, "/api/admin/requests/obs-3", &auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["usage"], serde_json::Value::Null);
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/requests/does-not-exist",
        &auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "request_not_found");

    // 参数校验
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/requests?status_class=2xx&status_code=200",
        &auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_status_filter");
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/requests?status_class=6xx",
        &auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_status_filter");
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/requests?limit=0",
        &auth,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers"]
async fn catalog_control_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query(
        "TRUNCATE providers, model_aliases, model_groups, request_logs, usage_records CASCADE",
    )
    .execute(&pool)
    .await
    .expect("truncate catalog control tables");
    let provider_id = uuid::Uuid::new_v4();
    let account_a = uuid::Uuid::new_v4();
    let account_b = uuid::Uuid::new_v4();
    let model_a = uuid::Uuid::new_v4();
    let model_b = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO providers (id, name, kind, base_url) \
         VALUES ($1, 'CatalogControlProvider', 'openai_compatible', 'http://localhost/v1')",
    )
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("provider");
    for (id, name) in [(account_a, "primary"), (account_b, "backup")] {
        sqlx::query(
            "INSERT INTO provider_accounts \
             (id, provider_id, name, region, credential_encrypted, credential_hint, weight, priority) \
             VALUES ($1, $2, $3, '', '\\x00'::bytea, 'sk-…test', 100, 1)",
        )
        .bind(id)
        .bind(provider_id)
        .bind(name)
        .execute(&pool)
        .await
        .expect("account");
    }
    for (id, name, canonical) in [
        (model_a, "catalog-model-a", "canonical-a"),
        (model_b, "catalog-model-b", "canonical-b"),
    ] {
        sqlx::query(
            "INSERT INTO upstream_models (id, provider_id, name, canonical, capabilities) \
             VALUES ($1, $2, $3, $4, '[\"chat\"]'::jsonb)",
        )
        .bind(id)
        .bind(provider_id)
        .bind(name)
        .bind(canonical)
        .execute(&pool)
        .await
        .expect("model");
    }

    let (status, _, _) = call(&app, Method::GET, "/api/admin/model-groups", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "catalog-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, login) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({
            "email": "catalog-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = login["csrf_token"].as_str().expect("csrf").to_string();
    let admin = [("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())];
    let before: i64 = sqlx::query_scalar("SELECT version FROM catalog_state WHERE id = 1")
        .fetch_one(&pool)
        .await
        .expect("catalog version");

    // 无 CSRF 403
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &[("cookie", cookie.clone())],
        Some(json!({"client_model_name": "no-csrf", "canonical": "no-csrf"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 创建 active 组 + 嵌套部署
    let create_group = json!({
        "client_model_name": "catalog-group",
        "display_name": "Catalog Group",
        "canonical": "canonical-a",
        "description": "test",
        "capabilities": ["chat", "stream"],
        "protocols": ["openai_chat"],
        "status": "active",
        "deployments": [{
            "upstream_model_id": model_a,
            "provider_account_id": account_a,
            "priority": 1,
            "weight": 100
        }]
    });
    let (status, headers, group) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &admin,
        Some(create_group.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(group["client_model_name"], "catalog-group");
    assert_eq!(group["status"], "active");
    assert_eq!(group["deployment_count"], 1);
    assert_eq!(group["enabled_deployment_count"], 1);
    assert_eq!(group["publication_status"], serde_json::Value::Null);
    assert_eq!(group["tool_default_count"], 0);
    assert_eq!(group["deployments"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        group["deployments"][0]["provider_account_id"],
        json!(account_a)
    );
    assert_eq!(group["deployments"][0]["account_name"], "primary");
    let group_id = group["id"].as_str().expect("group id").to_string();
    let deployment_id = group["deployments"][0]["id"]
        .as_str()
        .expect("deployment id")
        .to_string();

    // 重复名 409
    let mut duplicate = create_group.clone();
    duplicate["client_model_name"] = json!("catalog-group");
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &admin,
        Some(duplicate),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "client_model_name_conflict");

    // 激活但无部署 400
    let empty = json!({
        "client_model_name": "empty-group",
        "canonical": "empty",
        "status": "active"
    });
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &admin,
        Some(empty),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "no_enabled_deployment");

    // 列表/详情/预览
    let (status, _, page) = call(
        &app,
        Method::GET,
        "/api/admin/model-groups?status=active&limit=10",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 1);
    let (status, _, detail) = call(
        &app,
        Method::GET,
        &format!("/api/admin/model-groups/{group_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["id"], group_id);
    let (status, _, preview) = call(
        &app,
        Method::GET,
        &format!("/api/admin/model-groups/{group_id}/route-preview"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["candidates"].as_array().map(Vec::len), Some(1));

    // canonical 不一致 400
    let (status, _, body) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/model-groups/{group_id}"),
        &admin,
        Some(json!({"canonical": "canonical-b"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "canonical_mismatch");

    // 部署 CRUD：第二组部署
    let (status, _, deployment) = call(
        &app,
        Method::POST,
        "/api/admin/deployments",
        &admin,
        Some(json!({
            "upstream_model_id": model_b,
            "provider_account_id": account_b,
            "priority": 2,
            "weight": 50
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{deployment}");
    let second_id = deployment["id"]
        .as_str()
        .expect("second deployment")
        .to_string();
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/deployments",
        &admin,
        Some(json!({
            "upstream_model_id": model_b,
            "provider_account_id": account_b
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "deployment_conflict");
    let (status, _, updated) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/deployments/{second_id}"),
        &admin,
        Some(json!({"priority": 3, "weight": 20})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["priority"], 3);
    assert_eq!(updated["weight"], 20);
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/deployments/{second_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(
        &app,
        Method::GET,
        &format!("/api/admin/deployments/{second_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // providers 删除引用扩展：部署存在时 409（跨模块不变量）
    let (status, _, body) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/providers/{provider_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "provider_in_use");
    assert!(body["references"]["deployments"].as_u64().unwrap_or(0) > 0);
    let (status, _, body) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/providers/{provider_id}/accounts/{account_a}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "provider_account_in_use");
    assert!(body["references"]["deployments"].as_u64().unwrap_or(0) > 0);
    let (status, _, body) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/providers/{provider_id}/models/{model_a}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "upstream_model_in_use");
    assert!(body["references"]["deployments"].as_u64().unwrap_or(0) > 0);

    // 归档组 + 归档后入组拒绝
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/model-groups/{group_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/deployments",
        &admin,
        Some(json!({
            "upstream_model_id": model_b,
            "provider_account_id": account_b,
            "model_group_id": group_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "model_group_not_active");
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/model-groups/{group_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // version bump + deployment 仍可读取
    let after: i64 = sqlx::query_scalar("SELECT version FROM catalog_state WHERE id = 1")
        .fetch_one(&pool)
        .await
        .expect("catalog version");
    assert!(after > before, "version {before} -> {after}");
    let (status, _, _) = call(
        &app,
        Method::GET,
        &format!("/api/admin/deployments/{deployment_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers"]
async fn catalog_publication_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query(
        "TRUNCATE admin_users, users, providers, model_aliases, model_groups, request_logs, usage_records CASCADE",
    )
    .execute(&pool)
    .await
    .expect("truncate catalog publication tables");
    let provider_id = uuid::Uuid::new_v4();
    let account_id = uuid::Uuid::new_v4();
    let model_id = uuid::Uuid::new_v4();
    let model2_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO providers (id, name, kind, base_url) \
         VALUES ($1, 'PubProvider', 'openai_compatible', 'http://localhost/v1')",
    )
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("provider");
    sqlx::query(
        "INSERT INTO provider_accounts \
         (id, provider_id, name, region, credential_encrypted, credential_hint, weight, priority) \
         VALUES ($1, $2, 'main', '', '\\x00'::bytea, 'sk-…test', 100, 1)",
    )
    .bind(account_id)
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("account");
    for (id, name) in [(model_id, "pub-model"), (model2_id, "pub-model-2")] {
        sqlx::query(
            "INSERT INTO upstream_models (id, provider_id, name, canonical, capabilities) \
             VALUES ($1, $2, $3, 'pub-canonical', '[\"chat\",\"stream\"]'::jsonb)",
        )
        .bind(id)
        .bind(provider_id)
        .bind(name)
        .execute(&pool)
        .await
        .expect("model");
    }

    // admin
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({"email": "pub-admin@corp.example.com", "password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, login) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({"email": "pub-admin@corp.example.com", "password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = login["csrf_token"].as_str().expect("csrf").to_string();
    let admin = [("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())];

    // group + deployment + publication
    let (status, _, group) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &admin,
        Some(json!({
            "client_model_name": "pub-group",
            "display_name": "Public Group",
            "canonical": "pub-canonical",
            "capabilities": ["chat", "stream"],
            "context_window": 65536,
            "max_output": 8192,
            "protocols": ["openai_chat"],
            "parameters": {"temperature": true},
            "status": "active",
            "deployments": [{"upstream_model_id": model_id, "provider_account_id": account_id}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let group_id = group["id"].as_str().expect("group id").to_string();
    let (status, _, publication) = call(
        &app,
        Method::POST,
        "/api/admin/publications",
        &admin,
        Some(json!({
            "model_group_id": group_id,
            "targets": [{"subject_type": "all_users", "visible": true, "allowed": true}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{publication}");
    let publication_id = publication["id"]
        .as_str()
        .expect("publication id")
        .to_string();
    assert_eq!(publication["model_group_id"], group_id);
    assert_eq!(publication["status"], "active");
    assert_eq!(publication["targets"].as_array().map(Vec::len), Some(1));
    assert_eq!(publication["targets"][0]["subject_type"], "all_users");
    assert_eq!(publication["enforcement"]["visible"], "enforced");

    // 重复发布 409
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/publications",
        &admin,
        Some(json!({
            "model_group_id": group_id,
            "targets": [{"subject_type": "all_users"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "publication_conflict");

    // 发布前置：draft/无部署组
    let (status, _, draft) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &admin,
        Some(json!({"client_model_name": "draft-group", "canonical": "draft"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let draft_id = draft["id"].as_str().expect("draft id").to_string();
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/publications",
        &admin,
        Some(json!({"model_group_id": draft_id, "targets": [{"subject_type": "all_users"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "model_group_not_ready");

    // 另一就绪组（无发布）用于 target 校验
    let (status, _, validation_group) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &admin,
        Some(json!({
            "client_model_name": "validation-group",
            "canonical": "pub-canonical",
            "status": "active",
            "deployments": [{
                "upstream_model_id": model2_id,
                "provider_account_id": account_id,
                "priority": 1,
                "weight": 100
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{validation_group}");
    let validation_group_id = validation_group["id"]
        .as_str()
        .expect("validation group id");

    // target 校验
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/publications",
        &admin,
        Some(json!({
            "model_group_id": validation_group_id,
            "targets": [{"subject_type": "application", "subject_id": uuid::Uuid::new_v4()}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "target_type_not_supported");
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/publications",
        &admin,
        Some(json!({
            "model_group_id": validation_group_id,
            "targets": [{"subject_type": "user", "subject_id": uuid::Uuid::new_v4()}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "user_not_found");
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/publications",
        &admin,
        Some(json!({
            "model_group_id": validation_group_id,
            "targets": [{"subject_type": "all_users", "visible": false, "allowed": true}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["code"] == "validation_failed" || body["code"] == "publication_target_invalid");

    // 客户端可见
    let (status, _, pair_code) = call(
        &app,
        Method::POST,
        "/api/admin/pairing-codes",
        &admin,
        Some(json!({"user_email": "pub-user@corp.example.com", "user_display_name": "Pub User"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let code = pair_code["pairing_code"]["code"].as_str().expect("code");
    let (status, _, paired) = call(
        &app,
        Method::POST,
        "/api/client/pair",
        &[],
        Some(json!({"code": code, "device": {"name": "Pub Mac", "platform": "macos", "client_version": "0.1.0"}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let alice_id = paired["user"]["id"].as_str().expect("alice id").to_string();
    let alice_auth = [
        (
            "authorization",
            format!("Bearer {}", paired["api_key"].as_str().expect("key")),
        ),
        (
            "x-gw-device-token",
            paired["token"].as_str().expect("token").to_string(),
        ),
    ];
    let (status, headers, list) = call(&app, Method::GET, "/v1/models", &alice_auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    let version = header_str(&headers, "x-gw-catalog-version").expect("version");
    assert!(version.starts_with("cat:"), "{version}");
    assert_eq!(list["catalog_version"], version);
    let data = list["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], "pub-group");
    assert_eq!(data[0]["display_name"], "Public Group");
    assert_eq!(data[0]["upstream_model"], "pub-model");
    assert_eq!(data[0]["protocols"], json!(["openai_chat"]));
    let serialized = serde_json::to_string(&list).expect("serialize");
    for forbidden in [
        "canonical",
        "price",
        "provider",
        "account",
        "allowed",
        "model_group_id",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "catalog response must not contain {forbidden}: {serialized}"
        );
    }
    let (status, headers, detail) =
        call(&app, Method::GET, "/v1/models/pub-group", &alice_auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "x-gw-catalog-version"), Some(version));
    assert_eq!(detail["parameters"]["temperature"], true);
    assert_eq!(detail["catalog_version"], version);

    // 他用户/Key 白名单
    let (status, _, pair_code) = call(
        &app,
        Method::POST,
        "/api/admin/pairing-codes",
        &admin,
        Some(json!({"user_email": "bob@corp.example.com", "user_display_name": "Bob"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let bob_code = pair_code["pairing_code"]["code"]
        .as_str()
        .expect("bob code");
    let (status, _, bob) = call(
        &app,
        Method::POST,
        "/api/client/pair",
        &[],
        Some(json!({"code": bob_code, "device": {"name": "Bob Mac", "platform": "macos", "client_version": "0.1.0"}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let bob_auth = [
        (
            "authorization",
            format!("Bearer {}", bob["api_key"].as_str().expect("key")),
        ),
        (
            "x-gw-device-token",
            bob["token"].as_str().expect("token").to_string(),
        ),
    ];
    // 全量替换为 user Alice
    let (status, _, updated) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/publications/{publication_id}"),
        &admin,
        Some(json!({
            "targets": [{"subject_type": "user", "subject_id": alice_id, "visible": true, "allowed": true}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["targets"][0]["subject_type"], "user");
    let (status, _, list) = call(&app, Method::GET, "/v1/models", &alice_auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["data"].as_array().map(Vec::len), Some(1));
    let (status, _, list) = call(&app, Method::GET, "/v1/models", &bob_auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["data"], json!([]));

    // Key 白名单
    let key_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM virtual_keys WHERE key_prefix = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(paired["api_key_prefix"].as_str().expect("prefix"))
    .fetch_one(&pool)
    .await
    .expect("key id");
    sqlx::query("UPDATE virtual_keys SET allowed_models = ARRAY['other'] WHERE id = $1")
        .bind(key_id)
        .execute(&pool)
        .await
        .expect("whitelist");
    let (status, _, list) = call(&app, Method::GET, "/v1/models", &alice_auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["data"], json!([]));
    sqlx::query("UPDATE virtual_keys SET allowed_models = ARRAY[]::text[] WHERE id = $1")
        .bind(key_id)
        .execute(&pool)
        .await
        .expect("reset whitelist");

    // 暂停/恢复/删除
    let (status, _, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/publications/{publication_id}"),
        &admin,
        Some(json!({"status": "paused"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, list) = call(&app, Method::GET, "/v1/models", &alice_auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["data"], json!([]));
    let (status, _, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/publications/{publication_id}"),
        &admin,
        Some(json!({"status": "active"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/publications/{publication_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, list) = call(&app, Method::GET, "/v1/models", &alice_auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["data"], json!([]));
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/publications/{publication_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers"]
async fn catalog_tool_defaults_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query(
        "TRUNCATE admin_users, users, providers, model_aliases, model_groups, request_logs, usage_records CASCADE",
    )
    .execute(&pool)
    .await
    .expect("truncate tool defaults tables");
    let provider_id = uuid::Uuid::new_v4();
    let account_id = uuid::Uuid::new_v4();
    let model_a = uuid::Uuid::new_v4();
    let model_b = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO providers (id, name, kind, base_url) \
         VALUES ($1, 'ToolProvider', 'openai_compatible', 'http://localhost/v1')",
    )
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("provider");
    sqlx::query(
        "INSERT INTO provider_accounts \
         (id, provider_id, name, region, credential_encrypted, credential_hint, weight, priority) \
         VALUES ($1, $2, 'main', '', '\\x00'::bytea, 'sk-…test', 100, 1)",
    )
    .bind(account_id)
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("account");
    for (id, name) in [(model_a, "tool-model-a"), (model_b, "tool-model-b")] {
        sqlx::query(
            "INSERT INTO upstream_models (id, provider_id, name, canonical, capabilities) \
             VALUES ($1, $2, $3, $3, '[\"chat\"]'::jsonb)",
        )
        .bind(id)
        .bind(provider_id)
        .bind(name)
        .execute(&pool)
        .await
        .expect("model");
    }

    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({"email": "tool-admin@corp.example.com", "password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, login) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({"email": "tool-admin@corp.example.com", "password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = login["csrf_token"].as_str().expect("csrf").to_string();
    let admin = [("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())];

    async fn active_group(
        app: &Router,
        admin: &[(&str, String)],
        name: &str,
        canonical: &str,
        model_id: uuid::Uuid,
        account_id: uuid::Uuid,
    ) -> String {
        let (status, _, group) = call(
            app,
            Method::POST,
            "/api/admin/model-groups",
            admin,
            Some(json!({
                "client_model_name": name,
                "canonical": canonical,
                "status": "active",
                "deployments": [{
                    "upstream_model_id": model_id,
                    "provider_account_id": account_id
                }]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group}");
        group["id"].as_str().expect("group id").to_string()
    }
    let group_a = active_group(
        &app,
        &admin,
        "tool-group-a",
        "tool-model-a",
        model_a,
        account_id,
    )
    .await;
    let group_b = active_group(
        &app,
        &admin,
        "tool-group-b",
        "tool-model-b",
        model_b,
        account_id,
    )
    .await;

    // draft 组用于 not_active
    let (status, _, draft) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &admin,
        Some(json!({"client_model_name": "tool-draft", "canonical": "tool-model-a"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let draft_id = draft["id"].as_str().expect("draft id");

    // 401/CSRF
    let (status, _, _) = call(
        &app,
        Method::GET,
        "/api/admin/tool-model-defaults",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/tool-model-defaults",
        &[("cookie", cookie.clone())],
        Some(json!({"tool_key": "claude_code", "scope_type": "global", "default_model_group_id": group_a})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 创建 global 默认 + 白名单
    let (status, headers, created) = call(
        &app,
        Method::POST,
        "/api/admin/tool-model-defaults",
        &admin,
        Some(json!({
            "tool_key": "claude_code",
            "scope_type": "global",
            "default_model_group_id": group_a,
            "allow_switch": true,
            "allowed_model_group_ids": [group_b]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(created["tool_key"], "claude_code");
    assert_eq!(created["scope_type"], "global");
    assert_eq!(created["scope_label"], "全部用户");
    assert_eq!(created["default_model_client_name"], "tool-group-a");
    assert_eq!(created["allow_switch"], true);
    assert_eq!(
        created["allowed_model_groups"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        created["allowed_model_groups"][0]["client_model_name"],
        "tool-group-b"
    );
    let default_id = created["id"].as_str().expect("default id").to_string();

    // 同作用域冲突 409
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/tool-model-defaults",
        &admin,
        Some(json!({
            "tool_key": "claude_code",
            "scope_type": "global",
            "default_model_group_id": group_b
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "tool_model_default_conflict");

    // 预留 scope 400
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/tool-model-defaults",
        &admin,
        Some(json!({
            "tool_key": "codex",
            "scope_type": "team",
            "default_model_group_id": group_a
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");

    // user 不存在 404
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/tool-model-defaults",
        &admin,
        Some(json!({
            "tool_key": "codex",
            "scope_type": "user",
            "scope_id": uuid::Uuid::new_v4(),
            "default_model_group_id": group_a
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "user_not_found");

    // 默认组非 active 400
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/tool-model-defaults",
        &admin,
        Some(json!({
            "tool_key": "codex",
            "scope_type": "global",
            "default_model_group_id": draft_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "model_group_not_active");

    // 列表 + 组详情聚合
    let (status, _, page) = call(
        &app,
        Method::GET,
        "/api/admin/tool-model-defaults?tool_key=claude_code&limit=10",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["id"], json!(default_id));
    let (status, _, group_detail) = call(
        &app,
        Method::GET,
        &format!("/api/admin/model-groups/{group_a}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(group_detail["tool_default_count"], 1);

    // PATCH：关闭切换并清空白名单
    let (status, _, updated) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/tool-model-defaults/{default_id}"),
        &admin,
        Some(json!({"allow_switch": false, "allowed_model_group_ids": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["allow_switch"], false);
    assert_eq!(updated["allowed_model_groups"], json!([]));

    // 非法/空 PATCH
    let (status, _, body) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/tool-model-defaults/{default_id}"),
        &admin,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");

    // DELETE 幂等
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/tool-model-defaults/{default_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/tool-model-defaults/{default_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(
        &app,
        Method::GET,
        &format!("/api/admin/tool-model-defaults/{default_id}"),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED); // 无 GET 详情 operation
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers"]
async fn catalog_preview_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query(
        "TRUNCATE admin_users, users, providers, model_aliases, model_groups, request_logs, usage_records CASCADE",
    )
    .execute(&pool)
    .await
    .expect("truncate preview tables");
    let provider_id = uuid::Uuid::new_v4();
    let account_id = uuid::Uuid::new_v4();
    let model_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO providers (id, name, kind, base_url) \
         VALUES ($1, 'PreviewProvider', 'openai_compatible', 'http://localhost/v1')",
    )
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("provider");
    sqlx::query(
        "INSERT INTO provider_accounts \
         (id, provider_id, name, region, credential_encrypted, credential_hint, weight, priority) \
         VALUES ($1, $2, 'main', '', '\\x00'::bytea, 'sk-…test', 100, 1)",
    )
    .bind(account_id)
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("account");
    sqlx::query(
        "INSERT INTO upstream_models (id, provider_id, name, canonical, capabilities) \
         VALUES ($1, $2, 'preview-model', 'preview-canonical', '[\"chat\"]'::jsonb)",
    )
    .bind(model_id)
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("model");

    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(
            json!({"email": "preview-admin@corp.example.com", "password": "change-me-please-123"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, login) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(
            json!({"email": "preview-admin@corp.example.com", "password": "change-me-please-123"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = login["csrf_token"].as_str().expect("csrf").to_string();
    let admin = [("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())];

    let (status, _, group) = call(
        &app,
        Method::POST,
        "/api/admin/model-groups",
        &admin,
        Some(json!({
            "client_model_name": "preview-group",
            "display_name": "Preview Group",
            "canonical": "preview-canonical",
            "status": "active",
            "protocols": ["openai_chat"],
            "deployments": [{"upstream_model_id": model_id, "provider_account_id": account_id}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let group_id = group["id"].as_str().expect("group id").to_string();
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/publications",
        &admin,
        Some(json!({
            "model_group_id": group_id,
            "targets": [{"subject_type": "all_users", "visible": true, "allowed": true}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/tool-model-defaults",
        &admin,
        Some(json!({
            "tool_key": "claude_code",
            "scope_type": "global",
            "default_model_group_id": group_id,
            "allow_switch": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // preview user + device + key
    let (status, _, pair_code) = call(
        &app,
        Method::POST,
        "/api/admin/pairing-codes",
        &admin,
        Some(json!({"user_email": "preview-user@corp.example.com", "user_display_name": "Preview User"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let code = pair_code["pairing_code"]["code"].as_str().expect("code");
    let (status, _, paired) = call(
        &app,
        Method::POST,
        "/api/client/pair",
        &[],
        Some(json!({"code": code, "device": {"name": "Preview Mac", "platform": "macos", "client_version": "0.1.0"}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = paired["user"]["id"].as_str().expect("user id").to_string();
    let device_id = paired["device"]["id"]
        .as_str()
        .expect("device id")
        .to_string();
    let key_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM virtual_keys WHERE key_prefix = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(paired["api_key_prefix"].as_str().expect("prefix"))
    .fetch_one(&pool)
    .await
    .expect("key id");

    let (status, _, _) = call(&app, Method::GET, "/api/admin/catalog/preview", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/admin/catalog/preview",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");
    let (status, _, body) = call(
        &app,
        Method::GET,
        &format!(
            "/api/admin/catalog/preview?user_id={}",
            uuid::Uuid::new_v4()
        ),
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "user_not_found");

    let uri = format!(
        "/api/admin/catalog/preview?user_id={user_id}&device_id={device_id}&key_id={key_id}"
    );
    let (status, headers, preview) =
        call(&app, Method::GET, &uri, &[("cookie", cookie.clone())], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert!(
        preview["catalog_version"]
            .as_str()
            .is_some_and(|v| v.starts_with("cat:"))
    );
    assert_eq!(preview["subject"]["user_id"], user_id);
    assert_eq!(preview["subject"]["device_id"], device_id);
    assert_eq!(preview["subject"]["key_id"], json!(key_id));
    assert_eq!(preview["subject"]["user_group_ids"], json!([]));
    let models = preview["models"].as_array().expect("models");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], "preview-group");
    assert_eq!(models[0]["visible"], true);
    assert_eq!(models[0]["allowed"], true);
    assert_eq!(models[0]["matched_by"], "all_users");
    assert!(models[0]["publication_id"].as_str().is_some());
    let defaults = preview["tool_defaults"].as_array().expect("defaults");
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0]["tool_key"], "claude_code");
    assert_eq!(defaults[0]["default_model"], "preview-group");
    assert_eq!(preview["enforcement"]["visible"], "enforced");
    let serialized = serde_json::to_string(&preview).expect("serialize");
    for forbidden in ["canonical", "price", "provider", "account"] {
        assert!(
            !serialized.contains(forbidden),
            "preview must not contain {forbidden}: {serialized}"
        );
    }
    // 预览不写 usage/log
    let usage: i64 = sqlx::query_scalar("SELECT count(*) FROM usage_records")
        .fetch_one(&pool)
        .await
        .expect("usage");
    assert_eq!(usage, 0);
    let logs: i64 = sqlx::query_scalar("SELECT count(*) FROM request_logs")
        .fetch_one(&pool)
        .await
        .expect("logs");
    assert_eq!(logs, 0);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE identity 表"]
async fn identity_password_change_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "password-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let login = |password: &'static str| {
        let app = app.clone();
        async move {
            call(
                &app,
                Method::POST,
                "/api/admin/auth/login",
                &[],
                Some(json!({"email": "password-admin@corp.example.com", "password": password})),
            )
            .await
        }
    };
    let (status, headers1, body1) = login("change-me-please-123").await;
    assert_eq!(status, StatusCode::OK);
    let cookie1 = cookie_pair(header_str(&headers1, "set-cookie").expect("cookie1"));
    let csrf1 = body1["csrf_token"].as_str().expect("csrf1").to_string();
    let (status, headers2, _) = login("change-me-please-123").await;
    assert_eq!(status, StatusCode::OK);
    let cookie2 = cookie_pair(header_str(&headers2, "set-cookie").expect("cookie2"));

    // 缺 CSRF 403
    let (status, _, _) = call(
        &app,
        Method::PATCH,
        "/api/admin/auth/password",
        &[("cookie", cookie1.clone())],
        Some(
            json!({"current_password": "change-me-please-123", "new_password": "new-password-456"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 当前密码错误 400，且密码不变
    let (status, _, body) = call(
        &app,
        Method::PATCH,
        "/api/admin/auth/password",
        &[("cookie", cookie1.clone()), ("x-csrf-token", csrf1.clone())],
        Some(json!({"current_password": "wrong-password-000", "new_password": "new-password-456"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");
    assert!(body["errors"]["current_password"].as_str().is_some());

    // 新旧相同 400
    let (status, _, _) = call(
        &app,
        Method::PATCH,
        "/api/admin/auth/password",
        &[("cookie", cookie1.clone()), ("x-csrf-token", csrf1.clone())],
        Some(json!({"current_password": "change-me-please-123", "new_password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 修改成功
    let (status, headers, _) = call(
        &app,
        Method::PATCH,
        "/api/admin/auth/password",
        &[("cookie", cookie1.clone()), ("x-csrf-token", csrf1.clone())],
        Some(
            json!({"current_password": "change-me-please-123", "new_password": "new-password-456"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));

    // 其他会话失效，当前会话仍有效
    let (status, _, _) = call(
        &app,
        Method::GET,
        "/api/admin/auth/me",
        &[("cookie", cookie2)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(
        &app,
        Method::GET,
        "/api/admin/auth/me",
        &[("cookie", cookie1)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 旧密码不可登录，新密码可登录
    let (status, _, _) = login("change-me-please-123").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = login("new-password-456").await;
    assert_eq!(status, StatusCode::OK);

    // 审计事件
    crate::modules::observability::service::flush(std::time::Duration::from_secs(2)).await;
    let row: (String, String, String) = sqlx::query_as(
        "SELECT action, actor_type, outcome FROM audit_logs \
         WHERE action = 'identity.auth.password_changed' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("password audit event");
    assert_eq!(row.0, "identity.auth.password_changed");
    assert_eq!(row.1, "admin");
    assert_eq!(row.2, "success");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE identity 表"]
async fn settings_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query(
        "INSERT INTO app_settings (id, default_provider_id, request_log_retention_days, \
         audit_retention_days, upstream_timeout_secs) VALUES (1, NULL, 30, 180, 60) \
         ON CONFLICT (id) DO UPDATE SET default_provider_id = NULL, \
         request_log_retention_days = 30, audit_retention_days = 180, \
         upstream_timeout_secs = 60, updated_at = now()",
    )
    .execute(&pool)
    .await
    .expect("restore app_settings");
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({
            "email": "settings-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, login) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({
            "email": "settings-admin@corp.example.com",
            "password": "change-me-please-123"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = login["csrf_token"].as_str().expect("csrf").to_string();

    let (status, _, _) = call(&app, Method::GET, "/api/admin/settings", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, headers, defaults) = call(
        &app,
        Method::GET,
        "/api/admin/settings",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(defaults["request_log_retention_days"], 30);
    assert_eq!(defaults["audit_retention_days"], 180);
    assert_eq!(defaults["upstream_timeout_secs"], 60);
    assert_eq!(defaults["default_provider_id"], serde_json::Value::Null);

    // 空 PATCH/非法范围 400
    let (status, _, body) = call(
        &app,
        Method::PATCH,
        "/api/admin/settings",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");
    for payload in [
        json!({"request_log_retention_days": 0}),
        json!({"audit_retention_days": 100}),
        json!({"upstream_timeout_secs": 999}),
    ] {
        let (status, _, _) = call(
            &app,
            Method::PATCH,
            "/api/admin/settings",
            &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
            Some(payload),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // 合法更新并回读
    let (status, headers, updated) = call(
        &app,
        Method::PATCH,
        "/api/admin/settings",
        &[("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())],
        Some(json!({
            "request_log_retention_days": 7,
            "audit_retention_days": 200,
            "upstream_timeout_secs": 30
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(updated["request_log_retention_days"], 7);
    assert_eq!(updated["audit_retention_days"], 200);
    assert_eq!(updated["upstream_timeout_secs"], 30);
    let (status, _, reread) = call(
        &app,
        Method::GET,
        "/api/admin/settings",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reread["request_log_retention_days"], 7);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE access 表"]
async fn user_groups_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query("TRUNCATE user_groups, users CASCADE")
        .execute(&pool)
        .await
        .expect("truncate user groups");
    let alice = uuid::Uuid::new_v4();
    let bob = uuid::Uuid::new_v4();
    for (id, email, name) in [
        (alice, "alice-group@corp.example.com", "Alice"),
        (bob, "bob-group@corp.example.com", "Bob"),
    ] {
        sqlx::query(
            "INSERT INTO users (id, email, display_name, status) VALUES ($1, $2, $3, 'active')",
        )
        .bind(id)
        .bind(email)
        .bind(name)
        .execute(&pool)
        .await
        .expect("seed user");
    }
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(json!({"email": "groups-admin@corp.example.com", "password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, login) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(json!({"email": "groups-admin@corp.example.com", "password": "change-me-please-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = login["csrf_token"].as_str().expect("csrf").to_string();
    let admin = [("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())];

    let (status, _, _) = call(&app, Method::GET, "/api/admin/user-groups", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 创建（成员 Alice）
    let (status, headers, created) = call(
        &app,
        Method::POST,
        "/api/admin/user-groups",
        &admin,
        Some(json!({"name": "Engineering", "description": "R&D", "member_ids": [alice]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    assert_eq!(created["name"], "Engineering");
    assert_eq!(created["members"].as_array().map(Vec::len), Some(1));
    let group_id = created["id"].as_str().expect("group id").to_string();

    // 大小写唯一冲突
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/user-groups",
        &admin,
        Some(json!({"name": "engineering"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "user_group_name_conflict");

    // 未知成员 404
    let (status, _, body) = call(
        &app,
        Method::POST,
        "/api/admin/user-groups",
        &admin,
        Some(json!({"name": "Ops", "member_ids": [uuid::Uuid::new_v4()]})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "user_not_found");

    // 列表 q + 详情
    let (status, _, page) = call(
        &app,
        Method::GET,
        "/api/admin/user-groups?q=engineer&limit=10",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["member_count"], 1);

    // 成员全量替换
    let (status, _, updated) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/user-groups/{group_id}"),
        &admin,
        Some(json!({"member_ids": [bob]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["members"].as_array().map(Vec::len), Some(1));
    assert_eq!(updated["members"][0]["email"], "bob-group@corp.example.com");

    // 空 PATCH 400
    let (status, _, body) = call(
        &app,
        Method::PATCH,
        &format!("/api/admin/user-groups/{group_id}"),
        &admin,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation_failed");

    // 删除幂等
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/user-groups/{group_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/admin/user-groups/{group_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 审计动作（user_group.created）
    crate::modules::observability::service::flush(std::time::Duration::from_secs(2)).await;
    let action: String = sqlx::query_scalar(
        "SELECT action FROM audit_logs WHERE action = 'access.user_group.created' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("user group audit");
    assert_eq!(action, "access.user_group.created");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers"]
async fn catalog_suggestions_and_defaults_http_contract() {
    let Some((app, pool)) = test_app().await else {
        return;
    };
    sqlx::query(
        "TRUNCATE providers, model_groups, model_aliases, request_logs, usage_records, users CASCADE",
    )
    .execute(&pool)
    .await
    .expect("truncate suggestion tables");
    let provider_id = uuid::Uuid::new_v4();
    let account_a = uuid::Uuid::new_v4();
    let account_b = uuid::Uuid::new_v4();
    let model_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO providers (id, name, kind, base_url) \
         VALUES ($1, 'SuggestionProvider', 'openai_compatible', 'http://localhost/v1')",
    )
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("provider");
    for (id, name) in [(account_a, "a"), (account_b, "b")] {
        sqlx::query(
            "INSERT INTO provider_accounts \
             (id, provider_id, name, region, credential_encrypted, credential_hint, weight, priority) \
             VALUES ($1, $2, $3, '', '\\x00'::bytea, 'sk-…test', 100, 1)",
        )
        .bind(id)
        .bind(provider_id)
        .bind(name)
        .execute(&pool)
        .await
        .expect("account");
    }
    sqlx::query(
        "INSERT INTO upstream_models (id, provider_id, name, canonical, capabilities) \
         VALUES ($1, $2, 'suggestion-model', 'suggestion-canonical', '[\"chat\"]'::jsonb)",
    )
    .bind(model_id)
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("model");

    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/auth/setup",
        &[],
        Some(
            json!({"email": "suggest-admin@corp.example.com", "password": "change-me-please-123"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, headers, login) = call(
        &app,
        Method::POST,
        "/api/admin/auth/login",
        &[],
        Some(
            json!({"email": "suggest-admin@corp.example.com", "password": "change-me-please-123"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie_pair(header_str(&headers, "set-cookie").expect("cookie"));
    let csrf = login["csrf_token"].as_str().expect("csrf").to_string();
    let admin = [("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())];

    for account_id in [account_a, account_b] {
        let (status, _, body) = call(
            &app,
            Method::POST,
            "/api/admin/deployments",
            &admin,
            Some(json!({
                "upstream_model_id": model_id,
                "provider_account_id": account_id
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    // 建议列表
    let (status, _, page) = call(
        &app,
        Method::GET,
        "/api/admin/model-group-suggestions?limit=10",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 1);
    let suggestion = &page["items"][0];
    assert_eq!(suggestion["canonical"], "suggestion-canonical");
    assert_eq!(suggestion["recommendation"], "merge");
    assert_eq!(suggestion["deployment_count"], 2);
    let suggestion_key = suggestion["suggestion_key"]
        .as_str()
        .expect("suggestion key")
        .to_string();

    // 确认建组
    let (status, _, confirmation) = call(
        &app,
        Method::POST,
        &format!("/api/admin/model-group-suggestions/{suggestion_key}/confirm"),
        &admin,
        Some(json!({
            "action": "create_group",
            "client_model_name": "suggested-group",
            "display_name": "Suggested Group"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{confirmation}");
    assert_eq!(confirmation["decision"], "merged");
    assert_eq!(
        confirmation["moved_deployment_ids"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let group_id = confirmation["group"]["id"]
        .as_str()
        .expect("group id")
        .to_string();

    let (status, _, page) = call(
        &app,
        Method::GET,
        "/api/admin/model-group-suggestions",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 0);

    // 发布 + 全局工具默认
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/publications",
        &admin,
        Some(json!({
            "model_group_id": group_id,
            "targets": [{"subject_type": "all_users", "visible": true, "allowed": true}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = call(
        &app,
        Method::POST,
        "/api/admin/tool-model-defaults",
        &admin,
        Some(json!({
            "tool_key": "claude_code",
            "scope_type": "global",
            "default_model_group_id": group_id,
            "allow_switch": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // 客户端默认模型
    let (status, _, _body) = call(&app, Method::GET, "/v1/model-defaults", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, pair_code) = call(
        &app,
        Method::POST,
        "/api/admin/pairing-codes",
        &admin,
        Some(json!({"user_email": "defaults-user@corp.example.com", "user_display_name": "Defaults User"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let code = pair_code["pairing_code"]["code"].as_str().expect("code");
    let (status, _, paired) = call(
        &app,
        Method::POST,
        "/api/client/pair",
        &[],
        Some(json!({"code": code, "device": {"name": "Defaults Mac", "platform": "macos", "client_version": "0.1.0"}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let auth = [
        (
            "authorization",
            format!("Bearer {}", paired["api_key"].as_str().expect("key")),
        ),
        (
            "x-gw-device-token",
            paired["token"].as_str().expect("token").to_string(),
        ),
    ];
    let (status, headers, defaults) =
        call(&app, Method::GET, "/v1/model-defaults", &auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), Some("no-store"));
    let version = header_str(&headers, "x-gw-catalog-version").expect("version");
    assert!(version.starts_with("cat:"), "{version}");
    assert_eq!(defaults["object"], "list");
    assert_eq!(defaults["catalog_version"], version);
    let data = defaults["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["tool_key"], "claude_code");
    assert_eq!(data[0]["default_model"], "suggested-group");
    assert_eq!(data[0]["allow_switch"], true);
    assert_eq!(data[0]["switchable_models"], json!(["suggested-group"]));
    let serialized = serde_json::to_string(&defaults).expect("serialize");
    for forbidden in [
        "scope_id",
        "provider",
        "price",
        "allowed",
        "warnings",
        "model_group_id",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "model-defaults must not contain {forbidden}: {serialized}"
        );
    }
}
