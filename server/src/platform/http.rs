//! HTTP 层公共件：请求 ID、Cookie 读写。中间件装配在 `router.rs`。

use axum::http::{HeaderMap, HeaderName, header};
use std::sync::Arc;

pub fn request_id_header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

/// 从 `Cookie` 头中读取指定名字的原始值（不做 URL 解码；Session token 为 base64url）。
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| {
            let mut parts = pair.trim().splitn(2, '=');
            match (parts.next(), parts.next()) {
                (Some(key), Some(value)) if key.trim() == name => Some(value.trim().to_string()),
                _ => None,
            }
        })
        .next()
        .filter(|value| !value.is_empty())
}

/// 构造 Session Cookie：固定 Path/HttpOnly/SameSite=Lax，生产加 Secure。
pub fn session_cookie(name: &str, value: &str, max_age_secs: i64, secure: bool) -> String {
    let secure_part = if secure { "; Secure" } else { "" };
    format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure_part}")
}

/// 构造清除 Cookie 的 Set-Cookie 值。
pub fn clear_cookie(name: &str, secure: bool) -> String {
    let secure_part = if secure { "; Secure" } else { "" };
    format!("{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure_part}")
}

/// 共享上游 HTTP 客户端：连接池 + 建连超时 + 禁止重定向/环境代理。
/// 具体请求超时由 proxy 按 headers/流空闲/总时长分层控制，不在这里设死。
pub fn shared_client() -> Arc<reqwest::Client> {
    use std::time::Duration;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .pool_max_idle_per_host(64)
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .user_agent(concat!("gw-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("build shared reqwest client");
    Arc::new(client)
}

/// 读取 platform 请求 ID 中间件注入的 `x-request-id`。
pub fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(request_id_header())
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}
