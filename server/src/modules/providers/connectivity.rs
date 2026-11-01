//! 连通性测试：GET `{base_url}/models`，10s 总超时、3s 连接、不跟随重定向。
//!
//! 返回“结果型”错误码（`接口设计.md` §8）；可选 `model` 时校验上游目录（≤64KB，不落库/日志）。

use std::time::{Duration, Instant};

use reqwest::{Client, redirect::Policy};
use serde_json::Value;

const MAX_MODELS_BODY: usize = 64 * 1024;

#[derive(Debug)]
pub struct ProbeOutcome {
    pub ok: bool,
    pub latency_ms: u64,
    pub http_status: Option<u16>,
    pub error_code: Option<&'static str>,
    pub error_message: Option<String>,
}

impl ProbeOutcome {
    fn ok(latency_ms: u64, http_status: u16) -> Self {
        Self {
            ok: true,
            latency_ms,
            http_status: Some(http_status),
            error_code: None,
            error_message: None,
        }
    }

    fn failed(
        latency_ms: u64,
        http_status: Option<u16>,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            ok: false,
            latency_ms,
            http_status,
            error_code: Some(code),
            error_message: Some(message.into()),
        }
    }
}

pub async fn probe(base_url: &str, api_key: &str, model: Option<&str>) -> ProbeOutcome {
    let started = Instant::now();
    let client = match Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(3))
        .redirect(Policy::none())
        .user_agent(concat!("gw-server/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return ProbeOutcome::failed(
                elapsed_ms(started),
                None,
                "connect_error",
                format!("HTTP client 初始化失败: {error}"),
            );
        }
    };

    if let Err(message) = dns_preflight(base_url).await {
        return ProbeOutcome::failed(elapsed_ms(started), None, "dns_error", message);
    }

    let url = format!("{base_url}/models");
    let response = client.get(&url).bearer_auth(api_key).send().await;

    match response {
        Ok(response) => {
            let status = response.status().as_u16();
            let latency_ms = elapsed_ms(started);
            if !response.status().is_success() {
                let code = match status {
                    401 => "unauthorized",
                    403 => "forbidden",
                    404 => "not_found",
                    _ => "upstream_error",
                };
                return ProbeOutcome::failed(
                    latency_ms,
                    Some(status),
                    code,
                    format!("上游返回 HTTP {status}"),
                );
            }

            let Some(model) = model else {
                return ProbeOutcome::ok(latency_ms, status);
            };

            let mut response = response;
            let mut body = Vec::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        if body.len() + chunk.len() > MAX_MODELS_BODY {
                            return ProbeOutcome::failed(
                                latency_ms,
                                Some(status),
                                "upstream_error",
                                "上游 /models 响应超过 64KB，跳过模型目录校验",
                            );
                        }
                        body.extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(error) => {
                        return ProbeOutcome::failed(
                            elapsed_ms(started),
                            Some(status),
                            "upstream_error",
                            format!("读取上游 /models 失败: {error}"),
                        );
                    }
                }
            }

            if models_contain(&body, model) {
                ProbeOutcome::ok(latency_ms, status)
            } else {
                ProbeOutcome::failed(
                    latency_ms,
                    Some(status),
                    "not_found",
                    format!("模型 `{model}` 不在上游 /models 目录中"),
                )
            }
        }
        Err(error) => {
            let latency_ms = elapsed_ms(started);
            if error.is_timeout() {
                return ProbeOutcome::failed(latency_ms, None, "timeout", "请求超过 10s 未完成");
            }
            ProbeOutcome::failed(
                latency_ms,
                None,
                "connect_error",
                format!("请求上游失败: {error}"),
            )
        }
    }
}

/// DNS 预检：解析失败明确返回 `dns_error`（区分连接拒绝/超时）。
async fn dns_preflight(base_url: &str) -> Result<(), String> {
    let Some((host, port)) = host_port(base_url) else {
        return Ok(());
    };
    let lookup = tokio::net::lookup_host((host.as_str(), port));
    match tokio::time::timeout(Duration::from_secs(3), lookup).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(format!("DNS 解析失败: {error}")),
        Err(_) => Err("DNS 解析超时（3s）".to_string()),
    }
}

fn host_port(base_url: &str) -> Option<(String, u16)> {
    let (rest, default_port) = if let Some(rest) = base_url.strip_prefix("https://") {
        (rest, 443u16)
    } else {
        (base_url.strip_prefix("http://")?, 80u16)
    };
    let authority = rest.split('/').next()?;
    if authority.is_empty() {
        return None;
    }
    if let Some(inner) = authority.strip_prefix('[') {
        let (host, tail) = inner.split_once(']')?;
        let port = tail
            .strip_prefix(':')
            .and_then(|value| value.parse().ok())
            .unwrap_or(default_port);
        return Some((host.to_string(), port));
    }
    if let Some((host, port)) = authority.rsplit_once(':')
        && !host.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
    {
        return Some((host.to_string(), port.parse().ok()?));
    }
    Some((authority.to_string(), default_port))
}

fn models_contain(body: &[u8], model: &str) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    value
        .get("data")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id == model)
            })
        })
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parsing() {
        assert_eq!(
            host_port("https://api.deepseek.com/v1"),
            Some(("api.deepseek.com".to_string(), 443))
        );
        assert_eq!(
            host_port("http://127.0.0.1:8000/v1"),
            Some(("127.0.0.1".to_string(), 8000))
        );
        assert_eq!(
            host_port("http://[::1]:8080/v1"),
            Some(("::1".to_string(), 8080))
        );
        assert_eq!(host_port("ftp://example.com/v1"), None);
    }

    #[test]
    fn models_list_parsing() {
        let body = br#"{"data":[{"id":"deepseek-chat"},{"id":"deepseek-reasoner"}]}"#;
        assert!(models_contain(body, "deepseek-chat"));
        assert!(!models_contain(body, "unknown-model"));
        assert!(!models_contain(b"not-json", "deepseek-chat"));
        assert!(!models_contain(b"[]", "deepseek-chat"));
    }
}
