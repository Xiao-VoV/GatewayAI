//! 凭证与校验：Argon2id、token/CSRF、SHA-256、常量时间比较、邮箱/密码规则。
//!
//! 规则见 `接口设计.md` §1.5/§1.7、`技术设计.md` §4。

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use subtle::ConstantTimeEq;

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};

/// 邮箱：trim → ASCII 小写；结构 `local@domain.tld`；≤254；不允许空白/控制字符。
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

pub fn validate_email(raw: &str) -> Result<String, String> {
    let email = normalize_email(raw);
    if email.is_empty() {
        return Err("邮箱不能为空".to_string());
    }
    if !email.is_ascii() {
        return Err("邮箱仅支持 ASCII 字符".to_string());
    }
    if email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("邮箱不能包含空白或控制字符".to_string());
    }
    if email.len() > 254 {
        return Err("邮箱长度不能超过 254".to_string());
    }

    let mut parts = email.splitn(2, '@');
    let local = parts.next().unwrap_or_default();
    let domain = parts
        .next()
        .ok_or_else(|| "邮箱格式不正确（缺少 @）".to_string())?;
    if local.is_empty() || local.len() > 64 {
        return Err("邮箱格式不正确（local part 长度 1–64）".to_string());
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return Err("邮箱格式不正确（域名需包含点）".to_string());
    }
    if domain.split('.').any(|label| {
        label.is_empty()
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }) {
        return Err("邮箱格式不正确（域名非法）".to_string());
    }

    let local_ok = local.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '.' | '!'
                    | '#'
                    | '$'
                    | '%'
                    | '&'
                    | '\''
                    | '*'
                    | '+'
                    | '/'
                    | '='
                    | '?'
                    | '^'
                    | '_'
                    | '`'
                    | '{'
                    | '|'
                    | '}'
                    | '~'
                    | '-'
            )
    });
    if !local_ok {
        return Err("邮箱格式不正确（local part 含非法字符）".to_string());
    }

    Ok(email)
}

/// 初始化密码：10–128 个 Unicode 字符。
pub fn validate_setup_password(raw: &str) -> Result<(), String> {
    let len = raw.chars().count();
    if len < 10 {
        return Err("密码至少 10 个字符".to_string());
    }
    if len > 128 {
        return Err("密码不能超过 128 个字符".to_string());
    }
    Ok(())
}

/// 登录密码：1–128 个 Unicode 字符。
pub fn validate_login_password(raw: &str) -> Result<(), String> {
    let len = raw.chars().count();
    if len == 0 {
        return Err("密码不能为空".to_string());
    }
    if len > 128 {
        return Err("密码不能超过 128 个字符".to_string());
    }
    Ok(())
}

/// 显示名：trim 后 ≤64 字符；可省略（存空串）。
pub fn validate_display_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.chars().count() > 64 {
        return Err("显示名不能超过 64 个字符".to_string());
    }
    Ok(name.to_string())
}

/// setup token：trim 后非空、≤256 字节。
pub fn validate_setup_token(raw: &str) -> Result<String, String> {
    let token = raw.trim();
    if token.is_empty() {
        return Err("setup token 不能为空".to_string());
    }
    if token.len() > 256 {
        return Err("setup token 过长".to_string());
    }
    Ok(token.to_string())
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| error.to_string())
}

pub fn verify_password(password_hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// 未知邮箱时执行等价耗时的校验（评审 H1 防枚举）。
pub fn dummy_verify(password: &str) {
    static DUMMY_HASH: OnceLock<String> = OnceLock::new();
    let hash = DUMMY_HASH.get_or_init(|| {
        hash_password("gw-dummy-password-never-matches")
            .expect("dummy Argon2 hash generation must succeed")
    });
    let _ = verify_password(hash, password);
}

/// 32 字节 CSPRNG → base64url（43 字符，无 padding、无前缀）。
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Session token 的 SHA-256，小写 hex（64 字符）。
pub fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 常量时间比较（CSRF、setup token）。
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    left.as_bytes().ct_eq(right.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalized_and_validated() {
        assert_eq!(
            normalize_email("  Admin@Corp.Example.COM "),
            "admin@corp.example.com"
        );
        assert!(validate_email("admin@corp.example.com").is_ok());
        assert!(validate_email("a@b.c").is_ok());
        assert!(validate_email("no-at-sign").is_err());
        assert!(validate_email("a@b").is_err());
        assert!(validate_email("a b@corp.example.com").is_err());
        assert!(validate_email("ü@corp.example.com").is_err());
        assert!(validate_email("a@-bad.example.com").is_err());
    }

    #[test]
    fn password_length_rules() {
        assert!(validate_setup_password("1234567890").is_ok());
        assert!(validate_setup_password("123456789").is_err());
        assert!(validate_setup_password(&"a".repeat(129)).is_err());
        assert!(validate_login_password("1").is_ok());
        assert!(validate_login_password("").is_err());
    }

    #[test]
    fn display_name_trimmed() {
        assert_eq!(
            validate_display_name("  平台管理员  ").unwrap(),
            "平台管理员"
        );
        assert!(validate_display_name(&"字".repeat(65)).is_err());
    }

    #[test]
    fn tokens_are_43_char_base64url() {
        let token = generate_token();
        assert_eq!(token.len(), 43);
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(token, generate_token());
    }

    #[test]
    fn sha256_hex_is_64_lower_hex() {
        let hash = sha256_hex("gw_admin_session_token");
        assert_eq!(hash.len(), 64);
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_eq!(hash, sha256_hex("gw_admin_session_token"));
    }

    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
    }

    #[test]
    fn argon2_hash_roundtrip() {
        let hash = hash_password("change-me-please-123").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password(&hash, "change-me-please-123"));
        assert!(!verify_password(&hash, "wrong-password"));
        assert!(!verify_password("not-a-phc", "whatever"));
    }

    #[test]
    fn dense_dummy_hash_verification_never_matches() {
        // 保持可调用且返回 false 的语义（不断言耗时）。
        dummy_verify("anything");
    }
}
