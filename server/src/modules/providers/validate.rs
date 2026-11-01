//! providers 字段校验（URL/名称/Key/模型字段）。规则见 `接口设计.md` §1.6。

use super::model::KIND_OPENAI_COMPATIBLE;

pub fn validate_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name.chars().count() > 100 {
        return Err("名称不能超过 100 个字符".to_string());
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("名称不能包含控制字符".to_string());
    }
    Ok(name.to_string())
}

pub fn validate_base_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("base_url 不能为空".to_string());
    }
    if raw.chars().count() > 512 {
        return Err("base_url 不能超过 512 个字符".to_string());
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("base_url 不能包含空白/控制字符".to_string());
    }
    let rest = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .ok_or_else(|| "base_url 必须是 http/https".to_string())?;
    if rest.contains('?') || rest.contains('#') || rest.contains('@') {
        return Err("base_url 不允许 userinfo/query/fragment".to_string());
    }
    let rest = rest.trim_end_matches('/');
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, format!("/{path}")),
        None => (rest, String::new()),
    };
    if host.is_empty() {
        return Err("base_url 缺少 host".to_string());
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
    {
        return Err("base_url host 非法".to_string());
    }
    if path.is_empty() || path == "/" {
        return Err("base_url 必须包含版本/接口前缀（如 /v1）".to_string());
    }
    let scheme = if raw.starts_with("https://") {
        "https"
    } else {
        "http"
    };
    Ok(format!("{scheme}://{rest}"))
}

pub fn validate_description(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.chars().count() > 500 {
        return Err("description 不能超过 500 个字符".to_string());
    }
    if value.chars().any(|c| c.is_control()) {
        return Err("description 不能包含控制字符".to_string());
    }
    Ok(value.to_string())
}

pub fn validate_region(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.chars().count() > 64 {
        return Err("region 不能超过 64 个字符".to_string());
    }
    if value.chars().any(|c| c.is_control()) {
        return Err("region 不能包含控制字符".to_string());
    }
    Ok(value.to_string())
}

pub fn validate_api_key(raw: &str) -> Result<(), String> {
    if raw.is_empty() {
        return Err("api_key 不能为空".to_string());
    }
    if raw.len() > 4096 {
        return Err("api_key 不能超过 4096 字节".to_string());
    }
    if raw.chars().any(|c| c.is_control()) {
        return Err("api_key 不能包含控制字符".to_string());
    }
    Ok(())
}

pub fn validate_kind(raw: &str) -> Result<(), String> {
    if raw.trim() == KIND_OPENAI_COMPATIBLE {
        Ok(())
    } else {
        Err("S2 仅支持 openai_compatible 供应商".to_string())
    }
}

pub fn validate_status(raw: &str) -> Result<String, String> {
    match raw.trim() {
        "active" | "disabled" => Ok(raw.trim().to_string()),
        _ => Err("status 必须是 active/disabled".to_string()),
    }
}

pub fn validate_weight(value: Option<i32>) -> Result<i32, String> {
    match value.unwrap_or(100) {
        0..=100 => Ok(value.unwrap_or(100)),
        _ => Err("weight 必须在 0–100".to_string()),
    }
}

pub fn validate_priority(value: Option<i32>) -> Result<i32, String> {
    match value.unwrap_or(1) {
        1..=1000 => Ok(value.unwrap_or(1)),
        _ => Err("priority 必须在 1–1000".to_string()),
    }
}

pub fn validate_rpm_limit(value: Option<i32>) -> Result<Option<i32>, String> {
    match value {
        Some(v) if !(1..=1_000_000).contains(&v) => Err("rpm_limit 必须在 1–1000000".to_string()),
        other => Ok(other),
    }
}

pub fn validate_tpm_limit(value: Option<i64>) -> Result<Option<i64>, String> {
    match value {
        Some(v) if !(1..=1_000_000_000_000).contains(&v) => {
            Err("tpm_limit 必须在 1–1000000000000".to_string())
        }
        other => Ok(other),
    }
}

pub fn validate_model_name(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("模型名不能为空".to_string());
    }
    if value.chars().count() > 200 {
        return Err("模型名不能超过 200 个字符".to_string());
    }
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("模型名不能包含空白/控制字符".to_string());
    }
    Ok(value.to_string())
}

pub fn validate_canonical(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("canonical 不能为空".to_string());
    }
    if value.chars().count() > 100 {
        return Err("canonical 不能超过 100 个字符".to_string());
    }
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("canonical 不能包含空白/控制字符".to_string());
    }
    Ok(value.to_string())
}

pub fn validate_capabilities(values: &[String]) -> Result<Vec<String>, String> {
    if values.len() > 20 {
        return Err("capabilities 最多 20 项".to_string());
    }
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            return Err("capabilities 项不能为空".to_string());
        }
        if value.chars().count() > 32 {
            return Err("capabilities 项不能超过 32 个字符".to_string());
        }
        out.push(value.to_string());
    }
    Ok(out)
}

pub fn validate_window(value: Option<i32>, field: &str) -> Result<Option<i32>, String> {
    match value {
        Some(v) if !(1..=10_000_000).contains(&v) => Err(format!("{field} 必须在 1–10000000")),
        other => Ok(other),
    }
}

pub fn validate_price(value: Option<f64>, field: &str) -> Result<Option<f64>, String> {
    let Some(v) = value else { return Ok(None) };
    if !v.is_finite() || !(0.0..=1_000_000_000.0).contains(&v) {
        return Err(format!("{field} 必须在 0–1000000000"));
    }
    let scaled = (v * 1_000_000.0).round();
    if (scaled / 1_000_000.0 - v).abs() > 1e-9 {
        return Err(format!("{field} 最多 6 位小数"));
    }
    Ok(Some(v))
}

pub fn validate_currency(raw: &str) -> Result<String, String> {
    match raw.trim() {
        "CNY" | "USD" => Ok(raw.trim().to_string()),
        _ => Err("currency 必须是 CNY/USD".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_rules() {
        assert_eq!(
            validate_base_url("https://api.deepseek.com/v1/").unwrap(),
            "https://api.deepseek.com/v1"
        );
        assert!(validate_base_url("http://127.0.0.1:8000/v1").is_ok());
        assert!(validate_base_url("ftp://api.example.com/v1").is_err());
        assert!(validate_base_url("https://api.example.com").is_err());
        assert!(validate_base_url("https://user@api.example.com/v1").is_err());
        assert!(validate_base_url("https://api.example.com/v1?x=1").is_err());
        assert!(validate_base_url("https://api.example.com/v1#x").is_err());
    }

    #[test]
    fn names_and_limits() {
        assert_eq!(validate_name("  DeepSeek  ").unwrap(), "DeepSeek");
        assert!(validate_name("").is_err());
        assert!(validate_name(&"x".repeat(101)).is_err());
        assert_eq!(validate_weight(Some(0)).unwrap(), 0);
        assert!(validate_weight(Some(101)).is_err());
        assert!(validate_priority(Some(0)).is_err());
        assert!(validate_rpm_limit(Some(0)).is_err());
        assert!(validate_tpm_limit(Some(0)).is_err());
    }

    #[test]
    fn model_rules() {
        assert!(validate_model_name("deepseek-chat").is_ok());
        assert!(validate_capabilities(&["chat".to_string(), "stream".to_string()]).is_ok());
        assert!(validate_capabilities(&["".to_string()]).is_err());
        assert!(validate_window(Some(0), "context_window").is_err());
        assert!(validate_price(Some(0.5), "price_input").is_ok());
        assert!(validate_price(Some(f64::NAN), "price_input").is_err());
        assert!(validate_currency("EUR").is_err());
    }
}
