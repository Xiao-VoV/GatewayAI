//! 跨模块管理写审计：全局 middleware + `(method,path)` → action/resource 推导。
//!
//! 设计见 `设计文档（暂未开源）`：
//! - 仅审计 `/api/admin/` 下的 POST/PATCH/PUT/DELETE（GET 不审计）；
//! - login/setup 由 identity 显式审计、requests/audit 导出由 S5.1 服务写审计，middleware 跳过；
//! - 调 `next` 后按响应状态判定 outcome：`<400 success`、`401/403 denied`、其余 `failure`；
//! - 审计 detached 旁路（失败不阻塞、不改响应），detail 仅 method/path/status。
//!
//! 动作命名 `{domain}.{resource}.{verb}`（过去式）；未识别路径兜底
//! `admin.<resource>.<method_lower>`。审计写入的脱敏由 `record_audit_detached` 统一负责。

use std::net::{IpAddr, SocketAddr};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::Response,
};
use serde_json::json;
use uuid::Uuid;

use crate::modules::identity::middleware::admin_session_from_headers;
use crate::platform::{http::request_id_from_headers, state::AppState};

use super::{model::AuditEvent, service::record_audit_detached};

/// 管理台路径前缀（只审计该前缀下的写方法）。
const ADMIN_PREFIX: &str = "/api/admin/";
/// 专属审计路径（`/api/admin/` 之后部分），middleware 跳过以去重。
const SKIP_PATHS: [&str; 4] = [
    "auth/login",
    "auth/setup",
    "requests/export",
    "audit/export",
];

/// 一条管理写请求的审计映射（纯函数推导结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminAuditMapping {
    /// 事件动作，如 `providers.provider.created`。
    pub action: String,
    /// 资源类型，如 `provider`。
    pub resource_type: String,
    /// 路径最后一个 UUID 段（创建类请求通常为 None）。
    pub resource_id: Option<String>,
}

/// 全局管理写审计 middleware（`from_fn_with_state` 装配在 router 层）。
///
/// 非审计路径直接放行（不做额外 session 查询）；审计写入为 detached 旁路，
/// **不改变** handler 的响应/错误码。
pub async fn admin_audit_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let Some(mapping) = derive_admin_audit(method.as_str(), &path) else {
        return next.run(request).await;
    };

    let headers = request.headers().clone();
    let peer_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0.ip());
    let request_id = request_id_from_headers(&headers);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    // 进入 handler 前解析（旁路只读；无效/无会话按匿名，denied 也留痕）。
    let session = admin_session_from_headers(&state, &headers).await;

    let response = next.run(request).await;

    let outcome = audit_outcome(response.status());
    let client_ip = if state.config.trust_proxy_headers {
        forwarded_client_ip(&headers)
    } else {
        None
    }
    .or_else(|| peer_ip.map(|ip| ip.to_string()));
    let (actor_type, actor_id, actor_email) = match &session {
        Some(session) => (
            "admin",
            Some(session.user.id),
            Some(session.user.email.clone()),
        ),
        None => ("anonymous", None, None),
    };
    record_audit_detached(
        &state,
        AuditEvent {
            actor_type: actor_type.to_string(),
            actor_id,
            actor_email,
            action: mapping.action,
            resource_type: mapping.resource_type,
            resource_id: mapping.resource_id,
            outcome: outcome.to_string(),
            client_ip,
            user_agent,
            request_id,
            detail: json!({
                "method": method.as_str(),
                "path": path,
                "status": response.status().as_u16(),
            }),
        },
    );
    response
}

/// 响应状态 → 审计 outcome：`<400 success`、`401/403 denied`、其余 `failure`。
pub fn audit_outcome(status: StatusCode) -> &'static str {
    match status.as_u16() {
        100..=399 => "success",
        401 | 403 => "denied",
        _ => "failure",
    }
}

/// 推导管理写请求的审计映射；非写方法/非管理台路径/去重路径 → `None`。
pub fn derive_admin_audit(method: &str, path: &str) -> Option<AdminAuditMapping> {
    let default_verb = write_verb(method)?;
    let rest = path.strip_prefix(ADMIN_PREFIX)?;
    if rest.is_empty() || SKIP_PATHS.contains(&rest) {
        return None;
    }
    let segments: Vec<&str> = rest
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }
    // 资源 id：路径最后一个 UUID 段（`/keys/{id}/reset` 等仍取 id）。
    let resource_id = segments
        .iter()
        .rev()
        .find_map(|segment| Uuid::parse_str(segment).ok())
        .map(|id| id.to_string());
    let (domain, resource_type, verb) = derive_parts(&segments, method, default_verb);
    // identity 专属动作按注册表命名（`identity.auth.logout`），其余按 `{domain}.{resource}.{verb}`。
    let action = if (domain.as_str(), resource_type.as_str(), verb.as_str())
        == ("identity", "admin_session", "logout")
    {
        "identity.auth.logout".to_string()
    } else if rest == "auth/password" {
        "identity.auth.password_changed".to_string()
    } else if rest == "settings" {
        // US-024：固定动作名 platform.settings.updated（资源 app_settings）。
        "platform.settings.updated".to_string()
    } else {
        format!("{domain}.{resource_type}.{verb}")
    };
    let resource_type = if rest == "auth/password" {
        "admin_user".to_string()
    } else {
        resource_type
    };
    Some(AdminAuditMapping {
        action,
        resource_type,
        resource_id,
    })
}

/// 管理写方法 → 默认动词；非写方法返回 None。
fn write_verb(method: &str) -> Option<&'static str> {
    match method {
        "POST" => Some("created"),
        "PATCH" | "PUT" => Some("updated"),
        "DELETE" => Some("deleted"),
        _ => None,
    }
}

fn derive_parts(
    segments: &[&str],
    method: &str,
    default_verb: &'static str,
) -> (String, String, String) {
    let last = segments.last().copied().unwrap_or_default();
    let suffix_verb = match last {
        "test" => Some("tested"),
        "reset" => Some("reset"),
        _ => None,
    };
    let mapped = |domain: &str, resource: &str, verb: &str| {
        (domain.to_string(), resource.to_string(), verb.to_string())
    };
    match segments[0] {
        "providers" => {
            let resource = if segments.contains(&"accounts") {
                "account"
            } else if segments.contains(&"models") {
                "upstream_model"
            } else {
                "provider"
            };
            mapped("providers", resource, suffix_verb.unwrap_or(default_verb))
        }
        "users" => mapped("access", "user", default_verb),
        "keys" => {
            let verb = match (last, method) {
                ("reset", _) => "reset",
                (_, "DELETE") => "revoked",
                _ => default_verb,
            };
            mapped("access", "key", verb)
        }
        "devices" => mapped(
            "access",
            "device",
            if method == "DELETE" {
                "unbound"
            } else {
                default_verb
            },
        ),
        "pairing-codes" => mapped(
            "access",
            "pairing_code",
            if method == "DELETE" {
                "invalidated"
            } else {
                default_verb
            },
        ),
        "user-groups" => mapped("access", "user_group", default_verb),
        "model-groups" => mapped("catalog", "model_group", default_verb),
        "deployments" => mapped("catalog", "deployment", default_verb),
        "publications" => mapped("catalog", "publication", default_verb),
        "tool-model-defaults" => mapped("catalog", "tool_model_default", default_verb),
        "model-aliases" => mapped("routing", "alias", default_verb),
        "model-group-suggestions" => {
            let verb = match last {
                "confirm" => "confirmed",
                "reject" => "rejected",
                _ => default_verb,
            };
            mapped("catalog", "suggestion", verb)
        }
        "budgets" => mapped("policy", "budget", default_verb),
        // 平台设置（US-024）：固定名 platform.settings.updated，资源 app_settings。
        "settings" => mapped("platform", "app_settings", "updated"),
        "auth" if last == "logout" => mapped("identity", "admin_session", "logout"),
        // 未识别路径兜底：admin.<resource>.<method_lower>（默认审计未来端点）。
        _ => (
            "admin".to_string(),
            normalize_resource_segment(segments[0]),
            method.to_ascii_lowercase(),
        ),
    }
}

/// 兜底 resource 段归一：小写、非字母数字转 `_`、去首尾 `_`，空 → `other`。
fn normalize_resource_segment(raw: &str) -> String {
    let normalized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = normalized.trim_matches('_');
    if trimmed.is_empty() {
        "other".to_string()
    } else {
        trimmed.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Method, Request},
    };
    use chrono::Utc;
    use http_body_util::BodyExt;
    use serde_json::Value;
    use std::sync::Arc;
    use std::time::Duration;
    use tower::ServiceExt;

    use crate::platform::{config::Config, crypto::Crypto, state::AppState};

    fn mapping(method: &str, path: &str) -> (String, String, Option<String>) {
        let mapping = derive_admin_audit(method, path).expect("mapping");
        (mapping.action, mapping.resource_type, mapping.resource_id)
    }

    #[test]
    fn maps_providers_accounts_models_actions() {
        assert_eq!(
            mapping("POST", "/api/admin/providers"),
            (
                "providers.provider.created".to_string(),
                "provider".to_string(),
                None
            )
        );
        let provider_id = Uuid::new_v4();
        assert_eq!(
            mapping("PATCH", &format!("/api/admin/providers/{provider_id}")),
            (
                "providers.provider.updated".to_string(),
                "provider".to_string(),
                Some(provider_id.to_string())
            )
        );
        assert_eq!(
            mapping("POST", &format!("/api/admin/providers/{provider_id}/test")).0,
            "providers.provider.tested"
        );
        let account_id = Uuid::new_v4();
        assert_eq!(
            mapping(
                "DELETE",
                &format!("/api/admin/providers/{provider_id}/accounts/{account_id}")
            ),
            (
                "providers.account.deleted".to_string(),
                "account".to_string(),
                Some(account_id.to_string())
            )
        );
        assert_eq!(
            mapping(
                "POST",
                &format!("/api/admin/providers/{provider_id}/accounts/{account_id}/test")
            )
            .0,
            "providers.account.tested"
        );
        assert_eq!(
            mapping(
                "POST",
                &format!("/api/admin/providers/{provider_id}/models")
            )
            .0,
            "providers.upstream_model.created"
        );
        assert_eq!(
            mapping(
                "PATCH",
                &format!("/api/admin/providers/{provider_id}/models/{provider_id}")
            )
            .0,
            "providers.upstream_model.updated"
        );
    }

    #[test]
    fn maps_access_actions() {
        let id = Uuid::new_v4();
        assert_eq!(
            mapping("PATCH", &format!("/api/admin/users/{id}")).0,
            "access.user.updated"
        );
        assert_eq!(
            mapping("POST", "/api/admin/pairing-codes").0,
            "access.pairing_code.created"
        );
        assert_eq!(
            mapping("DELETE", &format!("/api/admin/pairing-codes/{id}")).0,
            "access.pairing_code.invalidated"
        );
        assert_eq!(
            mapping("DELETE", &format!("/api/admin/devices/{id}")).0,
            "access.device.unbound"
        );
        assert_eq!(
            mapping("PATCH", &format!("/api/admin/devices/{id}")).0,
            "access.device.updated"
        );
        assert_eq!(
            mapping("DELETE", &format!("/api/admin/keys/{id}")).0,
            "access.key.revoked"
        );
        assert_eq!(
            mapping("POST", &format!("/api/admin/keys/{id}/reset")).0,
            "access.key.reset"
        );
        assert_eq!(
            mapping("PATCH", &format!("/api/admin/keys/{id}")).0,
            "access.key.updated"
        );
    }

    #[test]
    fn maps_catalog_routing_policy_actions() {
        let id = Uuid::new_v4();
        for (path, expected) in [
            ("/api/admin/model-groups", "catalog.model_group.created"),
            ("/api/admin/deployments", "catalog.deployment.created"),
            ("/api/admin/publications", "catalog.publication.created"),
            (
                "/api/admin/tool-model-defaults",
                "catalog.tool_model_default.created",
            ),
            ("/api/admin/model-aliases", "routing.alias.created"),
        ] {
            assert_eq!(mapping("POST", path).0, expected);
        }
        assert_eq!(
            mapping("PATCH", &format!("/api/admin/model-groups/{id}")).0,
            "catalog.model_group.updated"
        );
        assert_eq!(
            mapping("DELETE", &format!("/api/admin/deployments/{id}")).0,
            "catalog.deployment.deleted"
        );
        assert_eq!(
            mapping("PATCH", &format!("/api/admin/publications/{id}")).0,
            "catalog.publication.updated"
        );
        assert_eq!(
            mapping("PATCH", &format!("/api/admin/tool-model-defaults/{id}")).0,
            "catalog.tool_model_default.updated"
        );
        assert_eq!(
            mapping("DELETE", &format!("/api/admin/model-aliases/{id}")).0,
            "routing.alias.deleted"
        );
        assert_eq!(
            mapping("PUT", &format!("/api/admin/budgets/{id}")),
            (
                "policy.budget.updated".to_string(),
                "budget".to_string(),
                Some(id.to_string())
            )
        );
        assert_eq!(
            mapping("DELETE", &format!("/api/admin/budgets/{id}")).0,
            "policy.budget.deleted"
        );
        assert_eq!(
            mapping("PATCH", "/api/admin/settings"),
            (
                "platform.settings.updated".to_string(),
                "app_settings".to_string(),
                None
            )
        );
    }

    #[test]
    fn maps_identity_logout_and_fallback() {
        assert_eq!(
            mapping("POST", "/api/admin/auth/logout"),
            (
                "identity.auth.logout".to_string(),
                "admin_session".to_string(),
                None
            )
        );
        assert_eq!(
            mapping("POST", "/api/admin/future-things"),
            (
                "admin.future_things.post".to_string(),
                "future_things".to_string(),
                None
            )
        );
        assert_eq!(
            mapping("DELETE", "/api/admin/Future Things/123"),
            (
                "admin.future_things.delete".to_string(),
                "future_things".to_string(),
                None
            )
        );
    }

    #[test]
    fn skips_reads_explicit_audit_and_non_admin_paths() {
        // GET 不审计。
        assert!(derive_admin_audit("GET", "/api/admin/providers").is_none());
        assert!(derive_admin_audit("HEAD", "/api/admin/providers").is_none());
        // identity 显式审计 / 导出服务已审计 → middleware 跳过。
        assert!(derive_admin_audit("POST", "/api/admin/auth/login").is_none());
        assert!(derive_admin_audit("POST", "/api/admin/auth/setup").is_none());
        assert!(derive_admin_audit("POST", "/api/admin/requests/export").is_none());
        assert!(derive_admin_audit("DELETE", "/api/admin/audit/export").is_none());
        // 非管理台路径不审计。
        assert!(derive_admin_audit("POST", "/v1/chat/completions").is_none());
        assert!(derive_admin_audit("POST", "/api/client/pair").is_none());
    }

    #[test]
    fn outcome_maps_status() {
        assert_eq!(audit_outcome(StatusCode::OK), "success");
        assert_eq!(audit_outcome(StatusCode::CREATED), "success");
        assert_eq!(audit_outcome(StatusCode::NO_CONTENT), "success");
        assert_eq!(audit_outcome(StatusCode::UNAUTHORIZED), "denied");
        assert_eq!(audit_outcome(StatusCode::FORBIDDEN), "denied");
        assert_eq!(audit_outcome(StatusCode::BAD_REQUEST), "failure");
        assert_eq!(audit_outcome(StatusCode::NOT_FOUND), "failure");
        assert_eq!(audit_outcome(StatusCode::CONFLICT), "failure");
        assert_eq!(audit_outcome(StatusCode::INTERNAL_SERVER_ERROR), "failure");
        assert_eq!(audit_outcome(StatusCode::SERVICE_UNAVAILABLE), "failure");
    }

    // ---------- S5.2 gated：全局审计 middleware + identity 显式审计 ----------

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
        Some(AppState::new(
            Arc::new(config),
            Some(pool),
            Arc::new(Crypto::from_key_bytes([13u8; 32])),
        ))
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

    async fn flush_audit() {
        assert!(
            crate::modules::observability::service::flush(Duration::from_secs(5)).await,
            "audit detached writes must drain"
        );
    }

    type LatestAudit = (String, Option<Uuid>, Option<String>, Option<String>, Value);

    async fn latest_audit(
        pool: &sqlx::PgPool,
        since: chrono::DateTime<Utc>,
        action: &str,
        outcome: &str,
    ) -> LatestAudit {
        sqlx::query_as(
            "SELECT actor_type, actor_id, actor_email, resource_id, detail FROM audit_logs \
             WHERE created_at >= $1 AND action = $2 AND outcome = $3 \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(since)
        .bind(action)
        .bind(outcome)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("missing audit action={action} outcome={outcome}: {error}"))
    }

    async fn count_audits(
        pool: &sqlx::PgPool,
        since: chrono::DateTime<Utc>,
        action: &str,
        outcome: &str,
    ) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM audit_logs \
             WHERE created_at >= $1 AND action = $2 AND outcome = $3",
        )
        .bind(since)
        .bind(action)
        .bind(outcome)
        .fetch_one(pool)
        .await
        .expect("count audits")
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向测试库（HTTP 全路由 + audit_logs；用例随机 id 并自清理）"]
    async fn admin_audit_middleware_and_identity_events() {
        let Some(state) = test_state_db().await else {
            return;
        };
        let app = crate::router::app(state.clone());
        let pool = state.db.as_ref().expect("pool").clone();
        let started = Utc::now();
        // setup 只能在无管理员时成功；测试库干净化（其他 gated 用例自带 truncate/建号）。
        sqlx::query("DELETE FROM admin_sessions")
            .execute(&pool)
            .await
            .expect("clear admin sessions");
        sqlx::query("DELETE FROM admin_users")
            .execute(&pool)
            .await
            .expect("clear admin users");

        let suffix = Uuid::new_v4();
        let email = format!("audit-hook-{suffix}@corp.example.com");
        let password = "change-me-please-123";
        let pairing_user_email = format!("audit-hook-user-{suffix}@corp.example.com");

        // identity setup 成功 → 显式审计 success（actor=新管理员）。
        let (status, _, body) = http_call(
            &app,
            Method::POST,
            "/api/admin/auth/setup",
            &[],
            Some(json!({"email": email, "password": password})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "setup body={body}");
        let setup_user: Value = serde_json::from_str(&body).expect("setup json");
        let admin_id =
            Uuid::parse_str(setup_user["id"].as_str().expect("admin id")).expect("admin uuid");
        flush_audit().await;
        let (actor_type, actor_id, actor_email, resource_id, _) =
            latest_audit(&pool, started, "identity.auth.setup", "success").await;
        assert_eq!(actor_type, "admin");
        assert_eq!(actor_id, Some(admin_id));
        assert_eq!(actor_email.as_deref(), Some(email.as_str()));
        assert_eq!(resource_id.as_deref(), Some(admin_id.to_string().as_str()));

        // setup 重复 → 403 + 显式审计 failure（匿名 + 尝试邮箱）。
        let (status, _, _) = http_call(
            &app,
            Method::POST,
            "/api/admin/auth/setup",
            &[],
            Some(json!({"email": email, "password": password})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        flush_audit().await;
        let (actor_type, actor_id, actor_email, _, detail) =
            latest_audit(&pool, started, "identity.auth.setup", "failure").await;
        assert_eq!(actor_type, "anonymous");
        assert_eq!(actor_id, None);
        assert_eq!(actor_email.as_deref(), Some(email.as_str()));
        assert_eq!(detail["error_code"], "setup_already_completed");

        // 登录失败 → 401 + 显式审计 failure（匿名 + 尝试邮箱，不含密码）。
        let (status, _, _) = http_call(
            &app,
            Method::POST,
            "/api/admin/auth/login",
            &[],
            Some(json!({"email": email, "password": "wrong-password-123"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        flush_audit().await;
        let (actor_type, actor_id, actor_email, _, detail) =
            latest_audit(&pool, started, "identity.auth.login", "failure").await;
        assert_eq!(actor_type, "anonymous");
        assert_eq!(actor_id, None);
        assert_eq!(actor_email.as_deref(), Some(email.as_str()));
        assert!(detail["error_code"].as_str().is_some());

        // 登录成功 → 200 + 显式审计 success；拿 cookie/CSRF 供后续写请求。
        let (status, headers, body) = http_call(
            &app,
            Method::POST,
            "/api/admin/auth/login",
            &[],
            Some(json!({"email": email, "password": password})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "login body={body}");
        let login_body: Value = serde_json::from_str(&body).expect("login json");
        let csrf = login_body["csrf_token"]
            .as_str()
            .expect("csrf token")
            .to_string();
        let cookie = header_str(&headers, "set-cookie")
            .expect("set-cookie")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_string();
        flush_audit().await;
        let (actor_type, actor_id, actor_email, _, detail) =
            latest_audit(&pool, started, "identity.auth.login", "success").await;
        assert_eq!(actor_type, "admin");
        assert_eq!(actor_id, Some(admin_id));
        assert_eq!(actor_email.as_deref(), Some(email.as_str()));
        assert!(detail.is_object());
        let auth = [("cookie", cookie.clone()), ("x-csrf-token", csrf.clone())];

        // 管理写成功 → middleware success（action/resource/detail）。
        let (status, _, _) = http_call(
            &app,
            Method::POST,
            "/api/admin/pairing-codes",
            &auth,
            Some(json!({"user_email": pairing_user_email, "expires_in_seconds": 600})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        flush_audit().await;
        let (actor_type, actor_id, actor_email, resource_id, detail) =
            latest_audit(&pool, started, "access.pairing_code.created", "success").await;
        assert_eq!(actor_type, "admin");
        assert_eq!(actor_id, Some(admin_id));
        assert_eq!(actor_email.as_deref(), Some(email.as_str()));
        assert_eq!(resource_id, None);
        assert_eq!(detail["method"], "POST");
        assert_eq!(detail["path"], "/api/admin/pairing-codes");
        assert_eq!(detail["status"], 201);
        // middleware 位于 request-id/ConnectInfo 层内：request_id 与 peer client_ip 应可采集。
        let (audit_request_id, audit_client_ip): (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT request_id, client_ip FROM audit_logs \
             WHERE created_at >= $1 AND action = 'access.pairing_code.created' AND outcome = 'success' \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(started)
        .fetch_one(&pool)
        .await
        .expect("audit request context");
        assert!(audit_request_id.as_deref().is_some_and(|id| !id.is_empty()));
        assert_eq!(audit_client_ip.as_deref(), Some("127.0.0.1"));

        // CSRF 失败 → 403 denied（actor 已识别=admin）。
        let (status, _, _) = http_call(
            &app,
            Method::POST,
            "/api/admin/pairing-codes",
            &[
                ("cookie", cookie.clone()),
                ("x-csrf-token", "wrong".to_string()),
            ],
            Some(json!({"user_email": pairing_user_email, "expires_in_seconds": 600})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        flush_audit().await;
        let (actor_type, actor_id, _, _, detail) =
            latest_audit(&pool, started, "access.pairing_code.created", "denied").await;
        assert_eq!(actor_type, "admin");
        assert_eq!(actor_id, Some(admin_id));
        assert_eq!(detail["status"], 403);

        // 未登录写 → 401 denied（匿名）。
        let (status, _, _) = http_call(
            &app,
            Method::POST,
            "/api/admin/pairing-codes",
            &[],
            Some(json!({"user_email": pairing_user_email, "expires_in_seconds": 600})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        flush_audit().await;
        let (actor_type, actor_id, actor_email, _, detail) =
            latest_audit(&pool, started, "access.pairing_code.created", "denied").await;
        assert_eq!(actor_type, "anonymous");
        assert_eq!(actor_id, None);
        assert_eq!(actor_email, None);
        assert_eq!(detail["status"], 401);

        // 业务失败（400）→ failure + resource_id=路径最后 UUID。
        let user_id = Uuid::new_v4();
        let (status, _, _) = http_call(
            &app,
            Method::PATCH,
            &format!("/api/admin/users/{user_id}"),
            &auth,
            Some(json!({"bogus": true})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        flush_audit().await;
        let (actor_type, _, _, resource_id, detail) =
            latest_audit(&pool, started, "access.user.updated", "failure").await;
        assert_eq!(actor_type, "admin");
        assert_eq!(resource_id.as_deref(), Some(user_id.to_string().as_str()));
        assert_eq!(detail["status"], 400);

        // GET 不审计：调用列表接口后无对应 path 的审计行。
        let (status, _, _) = http_call(
            &app,
            Method::GET,
            "/api/admin/users",
            &[("cookie", cookie.clone())],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        flush_audit().await;
        let get_audits: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_logs \
             WHERE created_at >= $1 AND detail->>'path' = '/api/admin/users' AND detail->>'method' = 'GET'",
        )
        .bind(started)
        .fetch_one(&pool)
        .await
        .expect("get audits");
        assert_eq!(get_audits, 0, "GET must not be audited by middleware");

        // 导出不重复：S5.1 服务审计 1 条，middleware 跳过（无同 path 审计）。
        let (status, _, _) = http_call(
            &app,
            Method::GET,
            "/api/admin/requests/export",
            &[("cookie", cookie.clone())],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        flush_audit().await;
        assert_eq!(
            count_audits(&pool, started, "observability.request.exported", "success").await,
            1
        );
        let export_middleware_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_logs \
             WHERE created_at >= $1 AND detail->>'path' = '/api/admin/requests/export'",
        )
        .bind(started)
        .fetch_one(&pool)
        .await
        .expect("export middleware rows");
        assert_eq!(
            export_middleware_rows, 0,
            "export must not be double audited"
        );

        // logout 由 middleware 审计（session 已识别，随后被删除）。
        let (status, _, _) =
            http_call(&app, Method::POST, "/api/admin/auth/logout", &auth, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        flush_audit().await;
        let (actor_type, actor_id, _, _, _) =
            latest_audit(&pool, started, "identity.auth.logout", "success").await;
        assert_eq!(actor_type, "admin");
        assert_eq!(actor_id, Some(admin_id));

        // 清理：本测试窗口内产生的审计 + 管理员/session/pairing 用户。
        let _ = sqlx::query("DELETE FROM audit_logs WHERE created_at >= $1")
            .bind(started)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM admin_sessions")
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM admin_users").execute(&pool).await;
        let _ = sqlx::query("DELETE FROM pairing_codes")
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM users WHERE email = $1")
            .bind(&pairing_user_email)
            .execute(&pool)
            .await;
    }
}
