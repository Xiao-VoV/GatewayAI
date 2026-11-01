//! 请求日志脱敏/归一化（写入前二次防御，US-035）。
//!
//! 规则见 `设计文档（暂未开源）` §5.2：
//! - `path` 只接受路由模板（去 query；未知路径落 `/other`）；
//! - `user_agent` 去控制字符/折叠空白/截断 512 字符；
//! - `key_prefix` 仅接受 `sk-gw-` + 1–8 位字母数字；
//! - `client_ip` 只接受合法 `IpAddr`（不存端口）；
//! - `error_code` 仅接受 `^[a-z][a-z0-9_]{0,63}$`，否则 `gateway_error`；
//! - `request_id`/`trace_id` 仅接受 `^[A-Za-z0-9._:-]{1,128}$`（非法直接丢弃该条）；
//! - 审计 `detail` 递归剔除敏感键、整体序列化 ≤4KB，超限折叠为 `{"truncated":true}`。

use std::net::IpAddr;

use serde_json::{Map, Value};

/// UA 最大字符数（DB 设计：截断 512）。
pub const MAX_USER_AGENT_CHARS: usize = 512;
/// 模型/供应商名最大字符数。
pub const MAX_MODEL_NAME_CHARS: usize = 200;
/// request_id/trace_id 最大字符数。
pub const MAX_ID_CHARS: usize = 128;
/// 协议/适配器标识最大字符数。
pub const MAX_PROTOCOL_LABEL_CHARS: usize = 128;
/// 审计 detail 最大序列化字节数（技术设计 §5.2：≤4KB）。
pub const MAX_AUDIT_DETAIL_BYTES: usize = 4096;
/// 审计 detail 最大嵌套深度（防御性上限）。
const MAX_AUDIT_DETAIL_DEPTH: usize = 8;
/// 审计 detail 数组最多保留元素数（防御性上限；仍受 4KB 总限约束）。
const MAX_AUDIT_DETAIL_ITEMS: usize = 200;
/// 审计 detail 键名最大字符数。
const MAX_AUDIT_DETAIL_KEY_CHARS: usize = 64;
/// 审计 detail 敏感键名标记（不区分大小写、忽略 `_`/`-`/空白）。
const SENSITIVE_AUDIT_KEYS: [&str; 11] = [
    "password",
    "token",
    "key",
    "secret",
    "credential",
    "authorization",
    "cookie",
    "csrf",
    "body",
    "prompt",
    "content",
];

/// 允许的路由模板（S4 数据面 + 目录；真实路径参数不入库）。
const ROUTE_TEMPLATES: [&str; 4] = [
    "/v1/chat/completions",
    "/v1/messages",
    "/v1/responses",
    "/v1/models",
];

/// 路径归一：去 query string，仅保留已知路由模板；未知/Gemini 原生路径归模板。
pub fn sanitize_path(path: &str) -> String {
    let without_query = path.split('?').next().unwrap_or_default().trim();
    if without_query.is_empty() {
        return "/other".to_string();
    }
    if ROUTE_TEMPLATES.contains(&without_query) {
        return without_query.to_string();
    }
    // Gemini 原生路径（/v1beta/models/{model}:generateContent）只存模板前缀，
    // 不落真实路径参数。
    if without_query.starts_with("/v1beta/models") {
        return "/v1beta/models".to_string();
    }
    // catalog 目录详情（/v1/models/{...}）归一到目录模板。
    if without_query.starts_with("/v1/models/") {
        return "/v1/models".to_string();
    }
    "/other".to_string()
}

/// UA 归一：控制字符→空格、折叠连续空白、去首尾、截断 512 字符。
pub fn sanitize_user_agent(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    let mut last_was_space = false;
    for ch in raw.chars() {
        let ch = if ch.is_control() || ch.is_whitespace() {
            ' '
        } else {
            ch
        };
        if ch == ' ' {
            if last_was_space || out.is_empty() {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        if chars >= MAX_USER_AGENT_CHARS {
            break;
        }
        out.push(ch);
        chars += 1;
    }
    out.trim().to_string()
}

/// Key 前缀归一：仅接受 `sk-gw-` + 1–8 位 base64url 字符（字母/数字/`-`/`_`），否则 None。
///
/// access 生成的前缀是 `sk-gw-` + 6 位 base64url（`KEY_PREFIX_CHARS=12`），
/// 因此必须允许 `-`/`_`；仍只保留前缀、永不写入完整 Key。
pub fn sanitize_key_prefix(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let suffix = trimmed.strip_prefix("sk-gw-")?;
    if suffix.is_empty()
        || suffix.len() > 8
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// problem code 归一：`^[a-z][a-z0-9_]{0,63}$`，否则 `gateway_error`。
pub fn sanitize_error_code(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut chars = trimmed.chars();
    let valid = trimmed.chars().count() <= 64
        && chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
    if valid {
        trimmed.to_string()
    } else {
        "gateway_error".to_string()
    }
}

/// 客户端 IP 归一：仅接受合法 `IpAddr`（无端口）；非法/空 → None。
pub fn sanitize_client_ip(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    raw.parse::<IpAddr>().ok().map(|ip| ip.to_string())
}

/// 模型名/供应商名等文本：去控制字符、去首尾空白、截断 `max_chars`；空 → None。
pub fn sanitize_optional_text(raw: Option<&str>, max_chars: usize) -> Option<String> {
    let raw = raw?.trim();
    let cleaned = sanitize_user_agent(raw);
    if cleaned.is_empty() {
        return None;
    }
    Some(truncate_chars(&cleaned, max_chars))
}

/// HTTP 方法：仅保留 ASCII 字母、统一大写、截断 16；空 → `UNKNOWN`。
pub fn sanitize_method(raw: &str) -> String {
    let method: String = raw
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .take(16)
        .collect::<String>()
        .to_ascii_uppercase();
    if method.is_empty() {
        "UNKNOWN".to_string()
    } else {
        method
    }
}

/// `request_id`/`trace_id` 合法性：1–128 个 `[A-Za-z0-9._:-]`。
pub fn is_valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_ID_CHARS
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
}

/// 审计 detail 脱敏：递归剔除敏感键，超 4KB 折叠为 `{"truncated":true}`。
///
/// 规则见技术设计 §5.2：只保留对象/标量摘要；敏感键（password/token/key/secret/
/// credential/authorization/cookie/csrf/body/prompt/content）在任意层级被剔除。
pub fn sanitize_audit_detail(value: Value) -> Value {
    let sanitized = sanitize_detail_value(value, 0);
    match serde_json::to_vec(&sanitized) {
        Ok(serialized) if serialized.len() <= MAX_AUDIT_DETAIL_BYTES => sanitized,
        _ => serde_json::json!({ "truncated": true }),
    }
}

fn sanitize_detail_value(value: Value, depth: usize) -> Value {
    if depth > MAX_AUDIT_DETAIL_DEPTH {
        return serde_json::json!({ "truncated": true });
    }
    match value {
        Value::Object(map) => {
            let mut sanitized = Map::new();
            for (key, value) in map {
                let key = sanitize_detail_key(&key);
                if key.is_empty() || is_sensitive_audit_key(&key) {
                    continue;
                }
                sanitized.insert(key, sanitize_detail_value(value, depth + 1));
            }
            Value::Object(sanitized)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .take(MAX_AUDIT_DETAIL_ITEMS)
                .map(|item| sanitize_detail_value(item, depth + 1))
                .collect(),
        ),
        // 标量原样保留（字符串由 JSON 序列化负责转义；总大小由 4KB 上限兜底）。
        scalar => scalar,
    }
}

fn sanitize_detail_key(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_AUDIT_DETAIL_KEY_CHARS)
        .collect();
    cleaned.trim().to_string()
}

fn is_sensitive_audit_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    SENSITIVE_AUDIT_KEYS
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_drops_query_and_unknown_paths() {
        assert_eq!(
            sanitize_path("/v1/chat/completions?stream=true"),
            "/v1/chat/completions"
        );
        assert_eq!(sanitize_path("/v1/messages"), "/v1/messages");
        assert_eq!(sanitize_path("/v1/responses"), "/v1/responses");
        assert_eq!(sanitize_path("/v1/models"), "/v1/models");
        assert_eq!(sanitize_path("/v1/models/deepseek-chat"), "/v1/models");
        assert_eq!(
            sanitize_path("/v1beta/models/gemini-2.0:generateContent?key=secret"),
            "/v1beta/models"
        );
        assert_eq!(sanitize_path("/v1/unknown"), "/other");
        assert_eq!(sanitize_path(""), "/other");
        assert_eq!(
            sanitize_path("/v1/chat/completions/../../etc/passwd"),
            "/other"
        );
    }

    #[test]
    fn user_agent_strips_controls_and_truncates() {
        assert_eq!(
            sanitize_user_agent("Claude-Code/1.2.3"),
            "Claude-Code/1.2.3"
        );
        assert_eq!(
            sanitize_user_agent("curl/8.0\r\nX-Injected: evil"),
            "curl/8.0 X-Injected: evil"
        );
        assert_eq!(sanitize_user_agent("  a\t\tb\n\nc  "), "a b c");
        assert_eq!(sanitize_user_agent(""), "");
        let long = "a".repeat(MAX_USER_AGENT_CHARS + 100);
        assert_eq!(
            sanitize_user_agent(&long).chars().count(),
            MAX_USER_AGENT_CHARS
        );
        // 控制字符不进入落库值。
        assert!(!sanitize_user_agent("a\u{0}b\u{7f}").contains('\u{0}'));
    }

    #[test]
    fn key_prefix_only_accepts_sk_gw_shapes() {
        assert_eq!(
            sanitize_key_prefix("sk-gw-abcd"),
            Some("sk-gw-abcd".to_string())
        );
        assert_eq!(sanitize_key_prefix("sk-gw-a"), Some("sk-gw-a".to_string()));
        assert_eq!(
            sanitize_key_prefix("sk-gw-abcdefgh"),
            Some("sk-gw-abcdefgh".to_string())
        );
        assert_eq!(sanitize_key_prefix("sk-gw-abcdefghi"), None);
        assert_eq!(sanitize_key_prefix("sk-gw-"), None);
        assert_eq!(sanitize_key_prefix("sk-gw-ab.cd"), None);
        assert_eq!(sanitize_key_prefix("sk-other-abcd"), None);
        assert_eq!(sanitize_key_prefix(""), None);
    }

    #[test]
    fn error_code_is_snake_case_whitelist() {
        assert_eq!(sanitize_error_code("budget_exceeded"), "budget_exceeded");
        assert_eq!(sanitize_error_code("gateway_error"), "gateway_error");
        assert_eq!(sanitize_error_code("Bad Code"), "gateway_error");
        assert_eq!(sanitize_error_code("1bad"), "gateway_error");
        assert_eq!(sanitize_error_code("bad-code"), "gateway_error");
        assert_eq!(sanitize_error_code(""), "gateway_error");
        assert_eq!(sanitize_error_code(&"a".repeat(65)), "gateway_error");
        assert_eq!(
            sanitize_error_code(&format!("a{}", "b".repeat(63))).len(),
            64
        );
    }

    #[test]
    fn client_ip_parses_addr_without_port() {
        assert_eq!(
            sanitize_client_ip(Some("10.0.3.7")),
            Some("10.0.3.7".to_string())
        );
        assert_eq!(sanitize_client_ip(Some("::1")), Some("::1".to_string()));
        assert_eq!(sanitize_client_ip(Some("10.0.3.7:8080")), None);
        assert_eq!(sanitize_client_ip(Some("not-an-ip")), None);
        assert_eq!(sanitize_client_ip(Some("")), None);
        assert_eq!(sanitize_client_ip(None), None);
    }

    #[test]
    fn optional_text_truncates_and_rejects_blank() {
        assert_eq!(
            sanitize_optional_text(Some(" fast "), 200),
            Some("fast".to_string())
        );
        assert_eq!(sanitize_optional_text(Some("   "), 200), None);
        assert_eq!(sanitize_optional_text(None, 200), None);
        assert_eq!(
            sanitize_optional_text(Some("abcdef"), 3),
            Some("abc".to_string())
        );
        assert_eq!(
            sanitize_optional_text(Some(&"a".repeat(300)), MAX_MODEL_NAME_CHARS)
                .expect("cleaned")
                .chars()
                .count(),
            MAX_MODEL_NAME_CHARS
        );
    }

    #[test]
    fn method_is_uppercased_and_bounded() {
        assert_eq!(sanitize_method("post"), "POST");
        assert_eq!(sanitize_method(" Get "), "GET");
        assert_eq!(sanitize_method(""), "UNKNOWN");
        assert_eq!(sanitize_method("DROP TABLE"), "DROPTABLE");
    }

    #[test]
    fn request_id_validation_rejects_unsafe_values() {
        assert!(is_valid_request_id("6a1c9e40-1f77-4a64-9d8e-0b9f2a5e6c11"));
        assert!(is_valid_request_id("trace:1.2_3-a"));
        assert!(!is_valid_request_id(""));
        assert!(!is_valid_request_id("has space"));
        assert!(!is_valid_request_id("semi;colon"));
        assert!(!is_valid_request_id("inject'quote"));
        assert!(!is_valid_request_id(&"a".repeat(129)));
        assert!(is_valid_request_id(&"a".repeat(128)));
    }

    #[test]
    fn audit_detail_removes_sensitive_keys_recursively() {
        let detail = serde_json::json!({
            "daily_limit": {"before": "5.000000", "after": "10.000000"},
            "password": "hunter2",
            "token": "secret-token",
            "api_key": "sk-gw-1234",
            "key_id": "2b8d",
            "authorization": "Bearer x",
            "cookie": "gw_admin_session=x",
            "csrf": "csrf-token",
            "body": {"prompt": "hi"},
            "resource": {"resource_id": "r-1", "content": "hidden"},
            "nested": [{"secret": "s", "ok": 1}, {"ok": 2}],
            "outcome": "success",
        });
        let sanitized = sanitize_audit_detail(detail);
        let object = sanitized.as_object().expect("object root");
        for key in [
            "password",
            "token",
            "api_key",
            "key_id",
            "authorization",
            "cookie",
            "csrf",
            "body",
        ] {
            assert!(
                !object.contains_key(key),
                "sensitive key {key} must be removed"
            );
        }
        assert_eq!(object["daily_limit"]["before"], "5.000000");
        assert_eq!(object["outcome"], "success");
        assert_eq!(object["nested"][0]["ok"], 1);
        assert_eq!(object["nested"][1]["ok"], 2);
        assert!(object["nested"][0].get("secret").is_none());
        assert!(object["resource"].get("content").is_none());
        // 整个 detail 序列化中不出现敏感值。
        let serialized = sanitized.to_string();
        for secret in [
            "hunter2",
            "secret-token",
            "sk-gw-1234",
            "Bearer x",
            "csrf-token",
        ] {
            assert!(!serialized.contains(secret), "secret {secret} leaked");
        }
    }

    #[test]
    fn audit_detail_over_4kb_folds_to_truncated_marker() {
        let detail = serde_json::json!({ "blob": "x".repeat(5000) });
        let sanitized = sanitize_audit_detail(detail);
        assert_eq!(sanitized, serde_json::json!({"truncated": true}));
    }

    #[test]
    fn audit_detail_keeps_small_objects_and_caps_too_deep_nesting() {
        let detail = serde_json::json!({"ok": true, "count": 3, "tags": ["a", "b"]});
        assert_eq!(sanitize_audit_detail(detail.clone()), detail);

        // 超过最大深度时折叠该节点（不 panic、不超限）。
        let mut deep = serde_json::json!("leaf");
        for _ in 0..12 {
            deep = serde_json::json!({ "level": deep });
        }
        let sanitized = sanitize_audit_detail(deep);
        assert!(sanitized.get("level").is_some());
    }
}
