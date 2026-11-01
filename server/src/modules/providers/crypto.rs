//! providers 凭证加解密、AAD 与掩码。
//!
//! - 密文格式由 `platform::crypto` 定义：`[0x01][nonce][ciphertext+tag]`
//! - AAD 绑定 account_id，防止密文跨账号搬运
//! - 明文只在内存中使用；Debug 输出被屏蔽

use uuid::Uuid;

use crate::platform::{crypto::Crypto, error::AppError};

/// 只在内存中持有明文凭证；Debug 不输出内容。
pub struct SecretString(String);

impl SecretString {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(***)")
    }
}

pub fn credential_aad(account_id: Uuid) -> Vec<u8> {
    format!("gw:provider-credential:v1:{account_id}").into_bytes()
}

pub fn seal_credential(
    crypto: &Crypto,
    account_id: Uuid,
    api_key: &str,
) -> Result<Vec<u8>, AppError> {
    crypto
        .seal(api_key.as_bytes(), &credential_aad(account_id))
        .map_err(|error| AppError::internal(format!("credential seal failed: {error}")))
}

pub fn open_credential(
    crypto: &Crypto,
    account_id: Uuid,
    sealed: &[u8],
) -> Result<SecretString, AppError> {
    let plaintext = crypto
        .open(sealed, &credential_aad(account_id))
        .map_err(|error| {
            tracing::error!(
                account_id = %account_id,
                error = %error,
                "provider credential decryption failed; check GW_MASTER_KEY / GW_MASTER_KEY_FILE"
            );
            AppError::credential_unavailable()
        })?;
    let text = String::from_utf8(plaintext).map_err(|_| {
        tracing::error!(account_id = %account_id, "provider credential is not valid UTF-8");
        AppError::credential_unavailable()
    })?;
    Ok(SecretString(text))
}

/// 掩码：≥8 字符 `前3…后4`；4–7 `…后2`；<4 `…`。
pub fn mask_api_key(api_key: &str) -> String {
    let chars: Vec<char> = api_key.chars().collect();
    match chars.len() {
        0 => String::new(),
        n if n >= 8 => {
            let head: String = chars[..3].iter().collect();
            let tail: String = chars[n - 4..].iter().collect();
            format!("{head}…{tail}")
        }
        4..=7 => {
            let tail: String = chars[chars.len() - 2..].iter().collect();
            format!("…{tail}")
        }
        _ => "…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_rules() {
        assert_eq!(mask_api_key("sk-1234abcd"), "sk-…abcd");
        assert_eq!(mask_api_key("abcdefgh"), "abc…efgh");
        assert_eq!(mask_api_key("abcd"), "…cd");
        assert_eq!(mask_api_key("abc"), "…");
        assert_eq!(mask_api_key(""), "");
    }

    #[test]
    fn seal_open_roundtrip_and_aad_binding() {
        let crypto = Crypto::from_key_bytes([9u8; 32]);
        let account_id = Uuid::new_v4();
        let sealed = seal_credential(&crypto, account_id, "sk-secret").unwrap();
        assert_eq!(
            open_credential(&crypto, account_id, &sealed)
                .unwrap()
                .expose(),
            "sk-secret"
        );
        // 不同 account_id 的 AAD 解不开
        assert!(open_credential(&crypto, Uuid::new_v4(), &sealed).is_err());
        // 不同主密钥解不开
        let other = Crypto::from_key_bytes([8u8; 32]);
        assert!(open_credential(&other, account_id, &sealed).is_err());
    }
}
