//! gated Postgres 集成测试：真实 access/providers/routing/policy/metering/observability +
//! 本地 mock 上游（ephemeral axum），覆盖 Chat/Messages 非流式/流式、预算、401、fallback。
//!
//! 运行方式（使用可清空的测试库）：
//! ```sh
//! GW_TEST_DATABASE_URL=postgres://gw:gw@127.0.0.1:5432/gw_proxy_test \
//!   cargo test --bin gw-server proxy::integration_tests -- --ignored --test-threads=1
//! ```

#![cfg(test)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::{
    access::{
        model::{ClientPairRequest, CreatePairingCodeRequest, DeviceInput, PatchKeyRequest},
        service as access_service,
    },
    policy::model::UpsertBudgetRequest,
    policy::service as policy_service,
    providers::{
        model::{CreateAccountRequest, CreateModelRequest, CreateProviderRequest},
        service as providers_service,
    },
    routing::{
        model::{CreateModelAliasRequest, TargetInput},
        service as routing_service,
    },
};
use crate::platform::{config::Config, crypto::Crypto, state::AppState};

use super::adapters::chat::ADAPTER_ID as CHAT_ADAPTER;
use super::adapters::messages::ADAPTER_ID as MESSAGES_ADAPTER;

const MODE_JSON: u8 = 0;
const MODE_SSE: u8 = 1;
const MODE_TOOL_JSON: u8 = 2;
const MODE_ERROR_500: u8 = 3;
const MODE_SSE_TOOL: u8 = 5;

struct MockState {
    mode: AtomicU8,
    requests: Mutex<Vec<MockRequest>>,
}

#[derive(Debug, Clone)]
struct MockRequest {
    authorization: Option<String>,
    body: Value,
}

struct MockUpstream {
    state: Arc<MockState>,
    base_url: String,
}

impl MockUpstream {
    fn set_mode(&self, mode: u8) {
        self.state.mode.store(mode, Ordering::SeqCst);
    }

    fn request_count(&self) -> usize {
        self.state.requests.lock().expect("requests lock").len()
    }

    fn requests(&self) -> Vec<MockRequest> {
        self.state.requests.lock().expect("requests lock").clone()
    }
}

async fn start_mock() -> MockUpstream {
    let state = Arc::new(MockState {
        mode: AtomicU8::new(MODE_JSON),
        requests: Mutex::new(Vec::new()),
    });
    let router = Router::new()
        .route("/v1/chat/completions", post(mock_handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream");
    let addr = listener.local_addr().expect("mock addr");
    let server_state = state.clone();
    tokio::spawn(async move {
        let _ = server_state; // keep Arc alive with the server task
        axum::serve(listener, router).await.expect("serve mock");
    });
    MockUpstream {
        state,
        base_url: format!("http://{addr}/v1"),
    }
}

async fn mock_handler(
    State(state): State<Arc<MockState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    state
        .requests
        .lock()
        .expect("requests lock")
        .push(MockRequest {
            authorization: headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(ToString::to_string),
            body: parsed,
        });
    match state.mode.load(Ordering::SeqCst) {
        MODE_SSE => sse_response(),
        MODE_SSE_TOOL => sse_tool_response(),
        MODE_TOOL_JSON => tool_json_response(),
        MODE_ERROR_500 => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":{"code":"server_error","message":"mock boom"}}"#,
        )
            .into_response(),
        _ => json_response(),
    }
}

fn json_response() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 0,
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
        })
        .to_string(),
    )
        .into_response()
}

fn tool_json_response() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion",
            "created": 0,
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "t1",
                        "type": "function",
                        "function": {"name": "Read", "arguments": "{\"path\":\"a\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 4, "completion_tokens": 5, "total_tokens": 9}
        })
        .to_string(),
    )
        .into_response()
}

fn sse_tool_response() -> Response {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"Bash\",\"arguments\":\"{\\\"cmd\\\":\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"ls\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":5}}\n\n",
        "data: [DONE]\n\n",
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        body,
    )
        .into_response()
}

fn sse_response() -> Response {
    let body = concat!(
        "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"你\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"好\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"id\":\"x\",\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
        "data: [DONE]\n\n",
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        body,
    )
        .into_response()
}

struct Env {
    state: AppState,
    api_key: String,
    device_token: String,
    user_id: Uuid,
    key_id: Uuid,
    alias: String,
    upstream_model_id: Uuid,
    provider_id: Uuid,
    account_id: Uuid,
}

async fn test_env() -> Option<(Env, MockUpstream)> {
    let url = std::env::var("GW_TEST_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let pool = PgPool::connect(&url).await.expect("connect test db");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    sqlx::query(
        "TRUNCATE admin_users, users, providers, model_aliases, model_groups, budgets, usage_records, request_logs CASCADE",
    )
    .execute(&pool)
    .await
    .expect("truncate test tables");

    let config = Config {
        bind: "127.0.0.1:0".parse().expect("addr"),
        database_url: Some(url),
        log_filter: "off".to_string(),
        run_migrations: false,
        cookie_secure: false,
        setup_token: None,
        usd_cny_rate: rust_decimal::Decimal::new(720, 2),
        trust_proxy_headers: false,
        proxy_max_body_bytes: 1024 * 1024,
        proxy_expose_provider_header: true,
        usage_retention_days: 180,
        rollup_retention_days: 1825,
        rollup_interval_secs: 300,
        request_log_retention_days: 30,
        audit_retention_days: 180,
        log_cleanup_interval_secs: 3600,
    };
    let state = AppState::new(
        Arc::new(config),
        Some(pool.clone()),
        Arc::new(Crypto::from_key_bytes([9u8; 32])),
    );

    let admin_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO admin_users (id, email, password_hash, display_name) VALUES ($1, $2, 'x', 'Admin')",
    )
    .bind(admin_id)
    .bind(format!("admin-{}@test.local", admin_id.simple()))
    .execute(&pool)
    .await
    .expect("insert admin");

    let pairing = access_service::create_pairing_code(
        &state,
        admin_id,
        CreatePairingCodeRequest {
            user_email: "alice@test.local".to_string(),
            user_display_name: Some("Alice".to_string()),
            expires_in_seconds: Some(600),
        },
    )
    .await
    .expect("create pairing code");
    let paired = access_service::pair_client(
        &state,
        ClientPairRequest {
            code: pairing.pairing_code.code,
            device: Some(DeviceInput {
                name: Some("Test Mac".to_string()),
                platform: Some("macos".to_string()),
                client_version: Some("0.0.0".to_string()),
            }),
        },
        "127.0.0.1",
    )
    .await
    .expect("pair client");
    let user_id = paired.user.id;
    let key_hash = crate::modules::access::security::sha256_hex(&paired.api_key);
    let key_id: Uuid = sqlx::query_scalar("SELECT id FROM virtual_keys WHERE key_hash = $1")
        .bind(&key_hash)
        .fetch_one(&pool)
        .await
        .expect("select key id");

    let mock = start_mock().await;
    let (provider, account) = providers_service::create_provider(
        &state,
        CreateProviderRequest {
            name: format!("Mock-{}", mock.base_url),
            kind: "openai_compatible".to_string(),
            base_url: mock.base_url.clone(),
            description: String::new(),
            account: CreateAccountRequest {
                name: "main".to_string(),
                api_key: "sk-upstream-test".to_string(),
                region: String::new(),
                weight: Some(100),
                priority: Some(1),
                rpm_limit: None,
                tpm_limit: None,
            },
        },
    )
    .await
    .expect("create provider");
    let account_id = account.id;
    let provider_id = provider.id;
    let upstream_model = providers_service::create_model(
        &state,
        provider.id,
        CreateModelRequest {
            name: "deepseek-chat".to_string(),
            canonical: "deepseek-chat".to_string(),
            capabilities: vec![
                "chat".to_string(),
                "stream".to_string(),
                "stream_usage".to_string(),
            ],
            context_window: Some(65536),
            max_output: Some(8192),
            price_input: Some(2.0),
            price_output: Some(8.0),
            currency: Some("CNY".to_string()),
        },
    )
    .await
    .expect("create upstream model");
    routing_service::create_model_alias(
        &state,
        CreateModelAliasRequest {
            alias: "fast".to_string(),
            enabled: Some(true),
            targets: vec![TargetInput {
                upstream_model_id: upstream_model.id,
                priority: Some(1),
                weight: Some(100),
                enabled: Some(true),
            }],
        },
    )
    .await
    .expect("create alias");
    policy_service::upsert_budget(
        &state,
        user_id,
        UpsertBudgetRequest {
            daily_limit: "5.000000".to_string(),
            currency: None,
        },
    )
    .await
    .expect("budget");

    Some((
        Env {
            state,
            api_key: paired.api_key,
            device_token: paired.token,
            user_id,
            key_id,
            alias: "fast".to_string(),
            upstream_model_id: upstream_model.id,
            provider_id,
            account_id,
        },
        mock,
    ))
}

async fn insert_catalog_group(
    pool: &sqlx::PgPool,
    name: &str,
    canonical: &str,
    status: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO model_groups (id, client_model_name, display_name, canonical, status) \
         VALUES ($1, $2, $2, $3, $4)",
    )
    .bind(id)
    .bind(name)
    .bind(canonical)
    .bind(status)
    .execute(pool)
    .await
    .expect("insert catalog group");
    id
}

async fn insert_catalog_deployment(pool: &sqlx::PgPool, group_id: Uuid, env: &Env) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO deployments \
         (id, model_group_id, upstream_model_id, provider_account_id, priority, weight, enabled) \
         VALUES ($1, $2, $3, $4, 1, 100, true)",
    )
    .bind(id)
    .bind(group_id)
    .bind(env.upstream_model_id)
    .bind(env.account_id)
    .execute(pool)
    .await
    .expect("insert catalog deployment");
    id
}

async fn publish_group(pool: &sqlx::PgPool, group_id: Uuid, visible: bool, allowed: bool) {
    let publication_id = Uuid::new_v4();
    sqlx::query("INSERT INTO publications (id, model_group_id, status) VALUES ($1, $2, 'active')")
        .bind(publication_id)
        .bind(group_id)
        .execute(pool)
        .await
        .expect("insert publication");
    sqlx::query(
        "INSERT INTO publication_targets \
         (id, publication_id, subject_type, subject_id, visible, allowed) \
         VALUES ($1, $2, 'all_users', NULL, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(publication_id)
    .bind(visible)
    .bind(allowed)
    .execute(pool)
    .await
    .expect("insert publication target");
}

async fn call(
    state: &AppState,
    path: &str,
    api_key: &str,
    device_token: &str,
    body: Value,
) -> (StatusCode, HeaderMap, Bytes) {
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {api_key}"))
        .header("X-GW-Device-Token", device_token)
        .body(Body::from(body.to_string()))
        .expect("build request");
    let response = crate::router::app(state.clone())
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    (status, headers, body)
}

async fn call_get(
    state: &AppState,
    path: &str,
    api_key: &str,
    device_token: &str,
) -> (StatusCode, HeaderMap, Bytes) {
    let request = axum::http::Request::builder()
        .method("GET")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {api_key}"))
        .header("X-GW-Device-Token", device_token)
        .body(Body::empty())
        .expect("build get request");
    let response = crate::router::app(state.clone())
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    (status, headers, body)
}

async fn call_goog(
    state: &AppState,
    path: &str,
    api_key: &str,
    device_token: &str,
    body: Value,
) -> (StatusCode, HeaderMap, Bytes) {
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header("x-goog-api-key", api_key)
        .header("X-GW-Device-Token", device_token)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("build gemini request");
    let response = crate::router::app(state.clone())
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    (status, headers, body)
}

async fn flush_telemetry() {
    crate::modules::metering::service::flush(Duration::from_secs(2)).await;
    crate::modules::observability::service::flush(Duration::from_secs(2)).await;
}

async fn usage_count(state: &AppState) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM usage_records")
        .fetch_one(state.db.as_ref().expect("db"))
        .await
        .expect("usage count")
}

async fn log_count(state: &AppState) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM request_logs")
        .fetch_one(state.db.as_ref().expect("db"))
        .await
        .expect("log count")
}

fn chat_body(model: &str, stream: bool) -> Value {
    json!({
        "model": model,
        "stream": stream,
        "messages": [{"role": "user", "content": "hi"}]
    })
}

fn messages_body(stream: bool) -> Value {
    json!({
        "model": "fast",
        "max_tokens": 256,
        "stream": stream,
        "messages": [{"role": "user", "content": "hi"}]
    })
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库（会 TRUNCATE 测试表）"]
async fn chat_non_stream_records_usage_and_log() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    let (status, headers, body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body(&env.alias, false),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("x-gw-resolved-model")
            .and_then(|value| value.to_str().ok()),
        Some("deepseek-chat")
    );
    assert_eq!(
        headers
            .get("x-gw-adapter")
            .and_then(|value| value.to_str().ok()),
        Some(CHAT_ADAPTER)
    );
    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("request id")
        .to_string();
    let response: Value = serde_json::from_slice(&body).expect("json response");
    assert_eq!(response["model"], "fast");

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer sk-upstream-test")
    );
    assert_eq!(requests[0].body["model"], "deepseek-chat");

    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let row = sqlx::query(
        "SELECT status, input_tokens, output_tokens, resolved_upstream_model, provider_id, is_stream, fallback, attempt_count, currency FROM usage_records WHERE request_id = $1",
    )
    .bind(&request_id)
    .fetch_one(pool)
    .await
    .expect("usage row");
    assert_eq!(row.get::<String, _>("status"), "success");
    assert_eq!(row.get::<i32, _>("input_tokens"), 3);
    assert_eq!(row.get::<i32, _>("output_tokens"), 2);
    assert_eq!(
        row.get::<String, _>("resolved_upstream_model"),
        "deepseek-chat"
    );
    assert!(!row.get::<bool, _>("is_stream"));
    assert!(!row.get::<bool, _>("fallback"));
    assert_eq!(row.get::<i32, _>("attempt_count"), 1);
    assert_eq!(row.get::<String, _>("currency"), "CNY");
    let log = sqlx::query(
        "SELECT status_code, protocol, adapter, is_stream, response_size, key_prefix FROM request_logs WHERE request_id = $1",
    )
    .bind(&request_id)
    .fetch_one(pool)
    .await
    .expect("request log");
    assert_eq!(log.get::<i32, _>("status_code"), 200);
    assert_eq!(log.get::<String, _>("protocol"), "chat_completions");
    assert_eq!(log.get::<String, _>("adapter"), CHAT_ADAPTER);
    assert!(!log.get::<bool, _>("is_stream"));
    assert!(log.get::<Option<String>, _>("key_prefix").is_some());
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn chat_stream_forwards_sse_and_records_usage() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_SSE);
    let (status, headers, body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body(&env.alias, true),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("\"content\":\"你\""));
    assert!(text.contains("data: [DONE]"));
    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let row =
        sqlx::query("SELECT status, is_stream, output_tokens, usage_source FROM usage_records")
            .fetch_one(pool)
            .await
            .expect("usage row");
    assert_eq!(row.get::<String, _>("status"), "success");
    assert!(row.get::<bool, _>("is_stream"));
    assert_eq!(row.get::<i32, _>("output_tokens"), 2);
    assert_eq!(row.get::<String, _>("usage_source"), "upstream");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn messages_non_stream_converts_tool_use() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_TOOL_JSON);
    let request = json!({
        "model": "fast",
        "max_tokens": 256,
        "tools": [{"name": "Read", "description": "read", "input_schema": {"type": "object"}}],
        "messages": [{"role": "user", "content": "read a.txt"}]
    });
    let (status, headers, body) = call(
        &env.state,
        "/v1/messages",
        &env.api_key,
        &env.device_token,
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("x-gw-adapter")
            .and_then(|value| value.to_str().ok()),
        Some(MESSAGES_ADAPTER)
    );
    let response: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(response["type"], "message");
    assert_eq!(response["stop_reason"], "tool_use");
    assert_eq!(response["content"][0]["type"], "tool_use");
    assert_eq!(response["content"][0]["name"], "Read");
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let chat_messages = requests[0].body["messages"].as_array().expect("messages");
    assert_eq!(chat_messages[0]["content"], "read a.txt");
    assert_eq!(requests[0].body["tools"][0]["function"]["name"], "Read");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn messages_stream_emits_anthropic_events() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_SSE);
    let (status, _headers, body) = call(
        &env.state,
        "/v1/messages",
        &env.api_key,
        &env.device_token,
        messages_body(true),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("event: message_start"));
    assert!(text.contains("event: content_block_start"));
    assert!(text.contains("event: content_block_delta"));
    assert!(text.contains("event: message_delta"));
    assert!(text.contains("event: message_stop"));
    assert!(text.contains("\"text\":\"你\""));
    // messages 前端没有 stream_options 字段；网关按 stream_usage 能力注入。
    let requests = mock.requests();
    assert_eq!(requests[0].body["stream_options"]["include_usage"], true);
    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let row = sqlx::query("SELECT status_code, is_stream, protocol FROM request_logs")
        .fetch_one(pool)
        .await
        .expect("log");
    assert_eq!(row.get::<String, _>("protocol"), "messages");
    assert!(row.get::<bool, _>("is_stream"));
    assert_eq!(usage_count(&env.state).await, 1);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn budget_exceeded_does_not_call_upstream() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    policy_service::upsert_budget(
        &env.state,
        env.user_id,
        UpsertBudgetRequest {
            daily_limit: "0.000001".to_string(),
            currency: None,
        },
    )
    .await
    .expect("budget");
    let (status, _headers, body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body(&env.alias, false),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "budget_exceeded");
    assert_eq!(problem["budget"]["currency"], "CNY");
    assert_eq!(
        mock.request_count(),
        0,
        "budget rejection must not call upstream"
    );
    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let row = sqlx::query("SELECT status, http_status, error_code FROM usage_records")
        .fetch_one(pool)
        .await
        .expect("usage");
    assert_eq!(row.get::<String, _>("status"), "rejected");
    assert_eq!(row.get::<i32, _>("http_status"), 429);
    assert_eq!(row.get::<String, _>("error_code"), "budget_exceeded");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn revoked_key_returns_401_and_only_request_log() {
    let Some((env, _mock)) = test_env().await else {
        return;
    };
    access_service::revoke_key(&env.state, env.key_id)
        .await
        .expect("revoke key");
    let (status, _headers, body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body(&env.alias, false),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "key_revoked");
    flush_telemetry().await;
    assert_eq!(usage_count(&env.state).await, 0);
    assert_eq!(log_count(&env.state).await, 1);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn retry_then_fallback_sets_header_and_counts_attempts() {
    let Some((env, _mock)) = test_env().await else {
        return;
    };
    // 失败上游（5xx）：作为 priority 1 的首候选。
    let bad = start_mock().await;
    bad.set_mode(MODE_ERROR_500);
    let (bad_provider, _account) = providers_service::create_provider(
        &env.state,
        CreateProviderRequest {
            name: "BadProvider".to_string(),
            kind: "openai_compatible".to_string(),
            base_url: bad.base_url.clone(),
            description: String::new(),
            account: CreateAccountRequest {
                name: "main".to_string(),
                api_key: "sk-bad".to_string(),
                region: String::new(),
                weight: Some(100),
                priority: Some(1),
                rpm_limit: None,
                tpm_limit: None,
            },
        },
    )
    .await
    .expect("bad provider");
    let bad_model = providers_service::create_model(
        &env.state,
        bad_provider.id,
        CreateModelRequest {
            name: "bad-chat".to_string(),
            canonical: "bad-chat".to_string(),
            capabilities: vec!["chat".to_string(), "stream".to_string()],
            context_window: None,
            max_output: None,
            price_input: None,
            price_output: None,
            currency: Some("CNY".to_string()),
        },
    )
    .await
    .expect("bad model");
    // 用新别名避免影响 env.alias；主候选用 bad，备候选用 env 的 mock 模型。
    let alias = routing_service::create_model_alias(
        &env.state,
        CreateModelAliasRequest {
            alias: "fallback-alias".to_string(),
            enabled: Some(true),
            targets: vec![
                TargetInput {
                    upstream_model_id: bad_model.id,
                    priority: Some(1),
                    weight: Some(100),
                    enabled: Some(true),
                },
                TargetInput {
                    upstream_model_id: env.upstream_model_id,
                    priority: Some(2),
                    weight: Some(100),
                    enabled: Some(true),
                },
            ],
        },
    )
    .await
    .expect("fallback alias");
    assert_eq!(alias.alias, "fallback-alias");
    let request = json!({
        "model": "fallback-alias",
        "messages": [{"role": "user", "content": "hi"}]
    });
    let (status, headers, _body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("x-gw-fallback")
            .and_then(|value| value.to_str().ok()),
        Some("true")
    );
    assert_eq!(bad.request_count(), 1);
    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let row = sqlx::query("SELECT fallback, attempt_count, status FROM usage_records")
        .fetch_one(pool)
        .await
        .expect("usage");
    assert!(row.get::<bool, _>("fallback"));
    assert_eq!(row.get::<i32, _>("attempt_count"), 2);
    assert_eq!(row.get::<String, _>("status"), "success");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn messages_unsupported_content_is_rejected_without_upstream_call() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    let request = json!({
        "model": "fast",
        "max_tokens": 256,
        "messages": [{
            "role": "user",
            "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "..."}}]
        }]
    });
    let (status, _headers, body) = call(
        &env.state,
        "/v1/messages",
        &env.api_key,
        &env.device_token,
        request,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "unsupported_content");
    assert_eq!(mock.request_count(), 0);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn catalog_published_model_routes_with_group_headers_and_source() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    let pool = env.state.db.as_ref().expect("db").clone();
    let group = insert_catalog_group(&pool, "group-model", "deepseek-chat", "active").await;
    let deployment = insert_catalog_deployment(&pool, group, &env).await;
    publish_group(&pool, group, true, true).await;

    let (status, headers, body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body("group-model", false),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("x-gw-model-group")
            .and_then(|value| value.to_str().ok()),
        Some("group-model")
    );
    assert!(
        headers
            .get("x-gw-catalog-version")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|version| version.starts_with("cat:"))
    );
    assert_eq!(
        headers
            .get("x-gw-resolved-model")
            .and_then(|value| value.to_str().ok()),
        Some("deepseek-chat")
    );
    let response: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(response["model"], "group-model");
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["model"], "deepseek-chat");
    flush_telemetry().await;
    let row = sqlx::query(
    "SELECT source, deployment_id, provider_id, resolved_upstream_model, status FROM usage_records",
)
.fetch_one(&pool)
.await
.expect("usage");
    assert_eq!(row.get::<String, _>("source"), "catalog");
    assert_eq!(row.get::<Uuid, _>("deployment_id"), deployment);
    assert_eq!(row.get::<Uuid, _>("provider_id"), env.provider_id);
    assert_eq!(
        row.get::<String, _>("resolved_upstream_model"),
        "deepseek-chat"
    );
    assert_eq!(row.get::<String, _>("status"), "success");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn catalog_unpublished_group_does_not_fallback_alias() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    let pool = env.state.db.as_ref().expect("db").clone();
    // 与 legacy alias 同名，但 catalog active 组未发布：必须 403，不得回退 alias。
    let group = insert_catalog_group(&pool, &env.alias, "deepseek-chat", "active").await;
    insert_catalog_deployment(&pool, group, &env).await;

    let (status, _headers, body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body(&env.alias, false),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "model_not_allowed");
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("未发布"))
    );
    assert_eq!(mock.request_count(), 0);
    flush_telemetry().await;
    let row = sqlx::query("SELECT status, error_code FROM usage_records")
        .fetch_one(&pool)
        .await
        .expect("usage");
    assert_eq!(row.get::<String, _>("status"), "rejected");
    assert_eq!(row.get::<String, _>("error_code"), "model_not_allowed");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn catalog_visible_only_group_is_403_with_visible_detail() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    let pool = env.state.db.as_ref().expect("db").clone();
    let group = insert_catalog_group(&pool, "visible-only", "deepseek-chat", "active").await;
    insert_catalog_deployment(&pool, group, &env).await;
    publish_group(&pool, group, true, false).await;

    let (status, _headers, body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body("visible-only", false),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "model_not_allowed");
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("可见"))
    );
    assert_eq!(mock.request_count(), 0);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn draft_catalog_group_does_not_shadow_alias() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    routing_service::create_model_alias(
        &env.state,
        CreateModelAliasRequest {
            alias: "draft-alias".to_string(),
            enabled: Some(true),
            targets: vec![TargetInput {
                upstream_model_id: env.upstream_model_id,
                priority: Some(1),
                weight: Some(100),
                enabled: Some(true),
            }],
        },
    )
    .await
    .expect("create alias");
    let pool = env.state.db.as_ref().expect("db").clone();
    insert_catalog_group(&pool, "draft-alias", "deepseek-chat", "draft").await;

    let (status, _headers, _body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body("draft-alias", false),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mock.request_count(), 1);
    flush_telemetry().await;
    let row = sqlx::query("SELECT source FROM usage_records")
        .fetch_one(&pool)
        .await
        .expect("usage");
    assert_eq!(row.get::<String, _>("source"), "alias");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn key_rpm_limit_third_request_429_without_upstream_or_usage() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    crate::modules::policy::rate_limit::reset_for_tests();
    access_service::update_key(
        &env.state,
        env.key_id,
        PatchKeyRequest {
            allowed_models: None,
            expires_at: None,
            rpm_limit: Some(Some(2)),
        },
    )
    .await
    .expect("set key rpm");

    for attempt in 0..2 {
        let (status, _headers, _body) = call(
            &env.state,
            "/v1/chat/completions",
            &env.api_key,
            &env.device_token,
            chat_body(&env.alias, false),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "request {attempt} should pass");
    }
    let (status, headers, body) = call(
        &env.state,
        "/v1/chat/completions",
        &env.api_key,
        &env.device_token,
        chat_body(&env.alias, false),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "key_rate_limited");
    assert!(!problem["detail"].as_str().unwrap_or_default().is_empty());
    let retry_after = headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .expect("Retry-After");
    assert!((1..=60).contains(&retry_after));
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+json")
    );
    assert_eq!(
        mock.request_count(),
        2,
        "rate-limited request must not reach upstream"
    );

    // 目录查询不占用/不受 Key RPM 限制。
    let (models_status, _headers, _body) =
        call_get(&env.state, "/v1/models", &env.api_key, &env.device_token).await;
    assert_eq!(models_status, StatusCode::OK);
    assert_eq!(mock.request_count(), 2);

    flush_telemetry().await;
    assert_eq!(usage_count(&env.state).await, 2, "429 must not write usage");
    // 3 次推理请求都写日志（含 429）；另有 1 条 /v1/models 目录日志（catalog 写、不受 RPM 限制）。
    assert_eq!(
        log_count(&env.state).await,
        4,
        "推理请求（含 429）均写 request_log，目录查询另记 1 条"
    );
    let pool = env.state.db.as_ref().expect("db");
    let row =
        sqlx::query("SELECT status_code, error_code FROM request_logs WHERE status_code = 429")
            .fetch_one(pool)
            .await
            .expect("429 request log");
    assert_eq!(row.get::<i32, _>("status_code"), 429);
    assert_eq!(row.get::<String, _>("error_code"), "key_rate_limited");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn responses_non_stream_text_records_usage_and_log() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    let (status, headers, body) = call(
        &env.state,
        "/v1/responses",
        &env.api_key,
        &env.device_token,
        json!({"model": env.alias, "input": "hi", "instructions": "be nice"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("x-gw-adapter")
            .and_then(|value| value.to_str().ok()),
        Some("responses_to_chat")
    );
    let response: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(response["object"], "response");
    assert_eq!(response["status"], "completed");
    assert_eq!(response["model"], "fast");
    assert_eq!(response["output"][0]["type"], "message");
    assert_eq!(response["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(response["output"][0]["content"][0]["text"], "ok");
    assert_eq!(response["usage"]["input_tokens"], 3);
    assert_eq!(response["usage"]["output_tokens"], 2);

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["model"], "deepseek-chat");
    assert_eq!(requests[0].body["messages"][0]["role"], "system");
    assert_eq!(requests[0].body["messages"][0]["content"], "be nice");
    assert_eq!(requests[0].body["messages"][1]["content"], "hi");

    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let usage = sqlx::query("SELECT status, input_tokens, output_tokens FROM usage_records")
        .fetch_one(pool)
        .await
        .expect("usage");
    assert_eq!(usage.get::<String, _>("status"), "success");
    assert_eq!(usage.get::<i32, _>("input_tokens"), 3);
    assert_eq!(usage.get::<i32, _>("output_tokens"), 2);
    let log = sqlx::query("SELECT protocol, adapter, upstream_protocol FROM request_logs")
        .fetch_one(pool)
        .await
        .expect("log");
    assert_eq!(log.get::<String, _>("protocol"), "responses");
    assert_eq!(log.get::<String, _>("adapter"), "responses_to_chat");
    assert_eq!(
        log.get::<String, _>("upstream_protocol"),
        "chat_completions"
    );
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn responses_non_stream_maps_function_call() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_TOOL_JSON);
    let (status, _headers, body) = call(
        &env.state,
        "/v1/responses",
        &env.api_key,
        &env.device_token,
        json!({
            "model": env.alias,
            "input": "read a file",
            "tools": [{"type": "function", "name": "Read", "description": "read", "parameters": {"type": "object"}}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let response: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"][0]["type"], "function_call");
    assert_eq!(response["output"][0]["call_id"], "t1");
    assert_eq!(response["output"][0]["name"], "Read");
    let requests = mock.requests();
    assert_tools_request(&requests);
}

fn assert_tools_request(requests: &[MockRequest]) {
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["tools"][0]["function"]["name"], "Read");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn responses_stream_text_emits_event_sequence() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_SSE);
    let (status, headers, body) = call(
        &env.state,
        "/v1/responses",
        &env.api_key,
        &env.device_token,
        json!({"model": env.alias, "input": "hi", "stream": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("event: response.created"));
    assert!(text.contains("event: response.in_progress"));
    assert!(text.contains("event: response.output_item.added"));
    assert!(text.contains("event: response.content_part.added"));
    assert!(text.contains("event: response.output_text.delta"));
    assert!(text.contains("event: response.output_text.done"));
    assert!(text.contains("event: response.content_part.done"));
    assert!(text.contains("event: response.output_item.done"));
    assert!(text.contains("event: response.completed"));
    assert!(!text.contains("data: [DONE]"));
    let requests = mock.requests();
    assert_eq!(requests[0].body["stream_options"]["include_usage"], true);
    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let usage = sqlx::query("SELECT status, is_stream FROM usage_records")
        .fetch_one(pool)
        .await
        .expect("usage");
    assert_eq!(usage.get::<String, _>("status"), "success");
    assert!(usage.get::<bool, _>("is_stream"));
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn responses_stream_function_call_emits_arguments_events() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_SSE_TOOL);
    let (status, _headers, body) = call(
        &env.state,
        "/v1/responses",
        &env.api_key,
        &env.device_token,
        json!({
            "model": env.alias,
            "input": "run ls",
            "stream": true,
            "tools": [{"type": "function", "name": "Bash", "parameters": {"type": "object"}}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("event: response.output_item.added"));
    assert!(text.contains("event: response.function_call_arguments.delta"));
    assert!(text.contains("event: response.function_call_arguments.done"));
    assert!(text.contains("event: response.output_item.done"));
    assert!(text.contains("event: response.completed"));
    assert!(text.contains("\"type\":\"function_call\""));
    assert!(text.contains("\"call_id\":\"call_1\""));
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn responses_validation_and_auth_are_problem_json() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    // 缺 input → 400 validation_failed。
    let (status, _headers, body) = call(
        &env.state,
        "/v1/responses",
        &env.api_key,
        &env.device_token,
        json!({"model": env.alias}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "validation_failed");

    // previous_response_id 无 input → 400 unsupported_parameter。
    let (status, _headers, body) = call(
        &env.state,
        "/v1/responses",
        &env.api_key,
        &env.device_token,
        json!({"model": env.alias, "previous_response_id": "resp_123"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "unsupported_parameter");

    // 无效 Key → 401。
    let (status, _headers, body) = call(
        &env.state,
        "/v1/responses",
        "sk-gw-invalid",
        &env.device_token,
        json!({"model": env.alias, "input": "hi"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "invalid_api_key");
    assert_eq!(mock.request_count(), 0);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn gemini_non_stream_text_with_goog_key_and_degraded_fields() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    let body = json!({
        "systemInstruction": {"parts": [{"text": "be terse"}]},
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
        "generationConfig": {"temperature": 0.2, "topP": 0.9, "maxOutputTokens": 64, "topK": 40},
        "safetySettings": [{"category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE"}]
    });
    let (status, headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:generateContent",
        &env.api_key,
        &env.device_token,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get("x-gw-adapter")
            .and_then(|value| value.to_str().ok()),
        Some("gemini_to_chat")
    );
    let degraded = headers
        .get("x-gw-degraded")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(degraded.contains("top_k"));
    assert!(degraded.contains("safety_settings"));
    let response: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        response["candidates"][0]["content"]["parts"][0]["text"],
        "ok"
    );
    assert_eq!(response["candidates"][0]["finishReason"], "STOP");
    assert_eq!(response["candidates"][0]["index"], 0);
    assert_eq!(response["usageMetadata"]["promptTokenCount"], 3);
    assert_eq!(response["modelVersion"], "fast");

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["model"], "deepseek-chat");
    assert_eq!(requests[0].body["messages"][0]["role"], "system");
    assert_eq!(requests[0].body["max_tokens"], 64);
    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let log = sqlx::query("SELECT protocol, adapter, upstream_protocol, path FROM request_logs")
        .fetch_one(pool)
        .await
        .expect("log");
    assert_eq!(log.get::<String, _>("protocol"), "gemini");
    assert_eq!(log.get::<String, _>("adapter"), "gemini_to_chat");
    assert_eq!(
        log.get::<String, _>("upstream_protocol"),
        "chat_completions"
    );
    let usage = sqlx::query("SELECT status FROM usage_records")
        .fetch_one(pool)
        .await
        .expect("usage");
    assert_eq!(usage.get::<String, _>("status"), "success");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn gemini_non_stream_function_call() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_TOOL_JSON);
    let body = json!({
        "contents": [{"role": "user", "parts": [{"text": "read a file"}]}],
        "tools": [{"functionDeclarations": [{"name": "Read", "description": "read", "parameters": {"type": "object"}}]}],
        "toolConfig": {"functionCallingConfig": {"mode": "ANY"}}
    });
    let (status, _headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:generateContent",
        &env.api_key,
        &env.device_token,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let response: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        response["candidates"][0]["content"]["parts"][0]["functionCall"]["name"],
        "Read"
    );
    let requests = mock.requests();
    assert_eq!(requests[0].body["tools"][0]["function"]["name"], "Read");
    assert_eq!(requests[0].body["tool_choice"], "required");
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn gemini_stream_text_alt_sse() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_SSE);
    let body = json!({
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
    });
    let (status, headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:streamGenerateContent?alt=sse",
        &env.api_key,
        &env.device_token,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("\"text\""));
    assert!(text.contains("\"finishReason\":\"STOP\""));
    assert!(text.contains("\"usageMetadata\""));
    assert!(!text.contains("[DONE]"));
    let requests = mock.requests();
    assert_eq!(requests[0].body["stream_options"]["include_usage"], true);
    flush_telemetry().await;
    let pool = env.state.db.as_ref().expect("db");
    let usage = sqlx::query("SELECT status, is_stream FROM usage_records")
        .fetch_one(pool)
        .await
        .expect("usage");
    assert_eq!(usage.get::<String, _>("status"), "success");
    assert!(usage.get::<bool, _>("is_stream"));
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn gemini_stream_function_call_final_chunk() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    mock.set_mode(MODE_SSE_TOOL);
    let body = json!({
        "contents": [{"role": "user", "parts": [{"text": "run ls"}]}],
        "tools": [{"functionDeclarations": [{"name": "Bash", "parameters": {"type": "object"}}]}]
    });
    let (status, _headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:streamGenerateContent?alt=sse",
        &env.api_key,
        &env.device_token,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("\"functionCall\""));
    assert!(text.contains("\"name\":\"Bash\""));
    assert!(text.contains("\"finishReason\":\"STOP\""));
    assert!(text.contains("\"usageMetadata\""));
    assert!(!text.contains("[DONE]"));
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn gemini_validation_route_and_auth_errors() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    let body = json!({"contents": []});
    let (status, _headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:generateContent",
        &env.api_key,
        &env.device_token,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "validation_failed");

    let body = json!({
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
        "generationConfig": {"candidateCount": 2}
    });
    let (status, _headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:generateContent",
        &env.api_key,
        &env.device_token,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "unsupported_parameter");

    // 流式必须 alt=sse。
    let (status, _headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:streamGenerateContent",
        &env.api_key,
        &env.device_token,
        json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "unsupported_parameter");

    // 未知动作 → 404 model_not_found。
    let (status, _headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:countTokens",
        &env.api_key,
        &env.device_token,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "model_not_found");

    // 缺 device token → 401。
    let (status, _headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:generateContent",
        &env.api_key,
        "",
        json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "missing_device_token");
    assert_eq!(mock.request_count(), 0);
}

#[tokio::test]
#[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库"]
async fn gemini_reuses_key_rpm_limit() {
    let Some((env, mock)) = test_env().await else {
        return;
    };
    crate::modules::policy::rate_limit::reset_for_tests();
    access_service::update_key(
        &env.state,
        env.key_id,
        PatchKeyRequest {
            allowed_models: None,
            expires_at: None,
            rpm_limit: Some(Some(1)),
        },
    )
    .await
    .expect("set key rpm");
    let body = json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]});
    let (status, _headers, _body) = call_goog(
        &env.state,
        "/v1beta/models/fast:generateContent",
        &env.api_key,
        &env.device_token,
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, headers, body) = call_goog(
        &env.state,
        "/v1beta/models/fast:generateContent",
        &env.api_key,
        &env.device_token,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let problem: Value = serde_json::from_slice(&body).expect("problem");
    assert_eq!(problem["code"], "key_rate_limited");
    assert!(
        headers
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .is_some()
    );
    assert_eq!(mock.request_count(), 1);
}
