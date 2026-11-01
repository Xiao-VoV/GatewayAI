//! access 凭证生成与校验：Crockford 配对码、device_secret/token、虚拟 Key 前缀。
//!
//! 复用 identity 的 SHA-256/随机 token/邮箱与显示名校验（同一项目内工具函数，不读写 identity 表）。

pub use super::model::KEY_PREFIX_CHARS;
pub use crate::modules::identity::security::{
    generate_token, sha256_hex, validate_display_name, validate_email,
};

/// Crockford base32 字符集（排除 I/L/O/U）。
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const PAIRING_CODE_LEN: usize = 8;
pub const API_KEY_PREFIX: &str = "sk-gw-";

/// 生成 8 位 Crockford 配对码，显示为 `XXXX-XXXX`。
pub fn generate_pairing_code() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut raw = String::with_capacity(PAIRING_CODE_LEN);
    for _ in 0..PAIRING_CODE_LEN {
        raw.push(CROCKFORD[rng.gen_range(0..CROCKFORD.len())] as char);
    }
    format!("{}-{}", &raw[..4], &raw[4..])
}

/// 规范化用户输入：去空格/横线、大写、O→0、I/L→1；返回 8 位裸码。
pub fn normalize_pairing_code(raw: &str) -> Result<String, String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect();
    if cleaned.len() != PAIRING_CODE_LEN {
        return Err(format!("配对码应为 {PAIRING_CODE_LEN} 位"));
    }
    if !cleaned
        .bytes()
        .all(|byte| CROCKFORD.contains(&byte.to_ascii_uppercase()))
    {
        return Err("配对码包含非法字符".to_string());
    }
    Ok(cleaned.to_ascii_uppercase())
}

pub fn hash_pairing_code(normalized: &str) -> String {
    sha256_hex(normalized)
}

/// 32 字节 base64url（43 字符）设备密钥。
pub fn generate_device_secret() -> String {
    generate_token()
}

/// 32 字节 base64url（43 字符）设备 token。
pub fn generate_device_token() -> String {
    generate_token()
}

/// 生成 `sk-gw-` + 32B base64url 的虚拟 Key；返回 (明文, 前缀)。
pub fn generate_api_key() -> (String, String) {
    let plain = format!("{API_KEY_PREFIX}{}", generate_token());
    let prefix: String = plain.chars().take(KEY_PREFIX_CHARS).collect();
    (plain, prefix)
}

pub fn validate_optional_text(
    raw: Option<&str>,
    field: &str,
    max_chars: usize,
) -> Result<String, String> {
    let value = raw.unwrap_or_default().trim();
    if value.chars().count() > max_chars {
        return Err(format!("{field} 不能超过 {max_chars} 个字符"));
    }
    Ok(value.to_string())
}

pub fn validate_email_address(raw: &str) -> Result<String, String> {
    validate_email(raw)
}

pub fn validate_user_display_name(raw: &str) -> Result<String, String> {
    validate_display_name(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_code_roundtrip_and_normalization() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), 9);
        assert_eq!(code.chars().filter(|c| *c == '-').count(), 1);
        let normalized = normalize_pairing_code(&code).unwrap();
        assert_eq!(normalized.len(), 8);
        assert_eq!(normalize_pairing_code("o1l2-3o4i").unwrap(), "01123041");
        assert!(normalize_pairing_code("short").is_err());
        assert!(normalize_pairing_code("!!!!-!!!!").is_err());
    }

    #[test]
    fn hash_is_stable_and_not_plaintext() {
        let normalized = "7Q2M9XKD";
        let hash = hash_pairing_code(normalized);
        assert_eq!(hash, hash_pairing_code(normalized));
        assert_eq!(hash.len(), 64);
        assert!(!hash.contains(normalized));
    }

    #[test]
    fn api_key_shape_and_prefix() {
        let (plain, prefix) = generate_api_key();
        assert!(plain.starts_with(API_KEY_PREFIX));
        assert_eq!(plain.len(), API_KEY_PREFIX.len() + 43);
        assert_eq!(prefix, plain[..12]);
        assert_eq!(prefix.len(), 12);
        assert_ne!(plain, generate_api_key().0);
    }

    #[test]
    fn tokens_are_random_43_chars() {
        let a = generate_device_secret();
        let b = generate_device_token();
        assert_eq!(a.len(), 43);
        assert_eq!(b.len(), 43);
        assert_ne!(a, b);
    }

    #[test]
    fn optional_text_trim_and_limit() {
        assert_eq!(
            validate_optional_text(Some("  MacBook  "), "name", 64).unwrap(),
            "MacBook"
        );
        assert!(validate_optional_text(Some(&"字".repeat(65)), "name", 64).is_err());
    }
}
