//! 主密钥加载与 AES-256-GCM 封装。
//!
//! 设计见 `设计文档（暂未开源）` §1.4：
//! - `GW_MASTER_KEY`（64 位 hex 或 44 字符 base64 → 32 字节）优先；
//! - 否则读 `GW_MASTER_KEY_FILE`（默认 `./data/master.key`，相对 server 工作目录）；
//! - 文件不存在时原子创建（0600，父目录 0700）；已存在权限过宽 → 启动失败；
//! - 格式正确但内容错误的密钥只能在首次解密时发现（返回明确错误，不降级明文）。
//!
//! 密文格式：`[0x01][nonce 12B][ciphertext+tag]`。

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::{Context, Result, bail};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use rand::{RngCore, rngs::OsRng};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;

const DEFAULT_KEY_FILE: &str = "./data/master.key";
const FORMAT_VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("ciphertext format invalid")]
    InvalidFormat,
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed（密文损坏或主密钥不匹配）")]
    Decrypt,
}

/// AES-256-GCM 主密钥封装器；只暴露 seal/open，不暴露密钥字节。
pub struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    pub fn from_key_bytes(key: [u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(&key)
                .expect("AES-256 key length is fixed at 32 bytes"),
        }
    }

    /// 从环境变量/密钥文件加载；启动失败带明确字段名（US-034）。
    pub fn load() -> Result<Self> {
        if let Ok(raw) = env::var("GW_MASTER_KEY") {
            let key = parse_key(&raw).context("GW_MASTER_KEY 无效")?;
            tracing::info!("master key loaded from GW_MASTER_KEY");
            return Ok(Self::from_key_bytes(key));
        }

        let path = env::var("GW_MASTER_KEY_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_KEY_FILE));
        let key = load_or_create_key_file(&path)?;
        Ok(Self::from_key_bytes(key))
    }

    pub fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| CryptoError::Encrypt)?;

        let mut out = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    pub fn open(&self, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if sealed.len() < 1 + NONCE_LEN + TAG_LEN || sealed[0] != FORMAT_VERSION {
            return Err(CryptoError::InvalidFormat);
        }
        let nonce = Nonce::from_slice(&sealed[1..1 + NONCE_LEN]);
        self.cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &sealed[1 + NONCE_LEN..],
                    aad,
                },
            )
            .map_err(|_| CryptoError::Decrypt)
    }
}

fn invalid_key(raw: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "主密钥必须是 32 字节：64 位 hex 或 44 字符 base64（当前长度 {}）",
        raw.len()
    )
}

fn parse_key(raw: &str) -> Result<[u8; 32]> {
    let raw = raw.trim();

    if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        let bytes = hex::decode(raw).context("主密钥 hex 解码失败")?;
        return bytes.try_into().map_err(|_| invalid_key(raw));
    }

    let decoded = STANDARD
        .decode(raw)
        .or_else(|_| URL_SAFE_NO_PAD.decode(raw))
        .map_err(|_| invalid_key(raw))?;
    decoded.try_into().map_err(|_| invalid_key(raw))
}

fn load_or_create_key_file(path: &Path) -> Result<[u8; 32]> {
    match fs::metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                bail!("GW_MASTER_KEY_FILE 不是普通文件: {}", path.display());
            }
            enforce_owner_only_permissions(path, &metadata)?;
            let bytes = fs::read(path)
                .with_context(|| format!("读取 GW_MASTER_KEY_FILE 失败: {}", path.display()))?;
            parse_key_file(&bytes, path)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_key_file(path),
        Err(error) => {
            Err(error).with_context(|| format!("访问 GW_MASTER_KEY_FILE 失败: {}", path.display()))
        }
    }
}

fn parse_key_file(bytes: &[u8], path: &Path) -> Result<[u8; 32]> {
    if bytes.len() == 32 {
        return Ok(bytes.try_into().expect("length checked"));
    }
    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("GW_MASTER_KEY_FILE 格式非法: {}", path.display()))?;
    parse_key(text).with_context(|| format!("GW_MASTER_KEY_FILE 内容非法: {}", path.display()))
}

fn create_key_file(path: &Path) -> Result<[u8; 32]> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_private_dir(parent)?;
    }

    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    match options.open(path) {
        Ok(mut file) => {
            file.write_all(&key)
                .with_context(|| format!("写入主密钥文件失败: {}", path.display()))?;
            file.sync_all().ok();
            tracing::warn!(
                path = %path.display(),
                "未找到主密钥，已生成新的 0600 密钥文件；请立即备份，丢失将导致所有上游凭证不可解密"
            );
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // 并发首次启动：另一实例已创建，复用其密钥。
            let bytes = fs::read(path)
                .with_context(|| format!("读取并发创建的主密钥文件失败: {}", path.display()))?;
            parse_key_file(&bytes, path)
        }
        Err(error) => Err(error).with_context(|| format!("创建主密钥文件失败: {}", path.display())),
    }
}

fn create_private_dir(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .with_context(|| format!("创建主密钥目录失败: {}", path.display()))
}

#[cfg(unix)]
fn enforce_owner_only_permissions(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "GW_MASTER_KEY_FILE 权限过宽（当前 {:o}，要求 0600）: {}",
            mode,
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn enforce_owner_only_permissions(_path: &Path, _metadata: &fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_crypto() -> Crypto {
        Crypto::from_key_bytes([7u8; 32])
    }

    #[test]
    fn seal_open_roundtrip() {
        let crypto = test_crypto();
        let aad = b"gw:provider-credential:v1:account";
        let sealed = crypto.seal(b"sk-test-key", aad).unwrap();
        assert_eq!(sealed[0], 1);
        assert_eq!(crypto.open(&sealed, aad).unwrap(), b"sk-test-key");
    }

    #[test]
    fn wrong_aad_or_tamper_fails() {
        let crypto = test_crypto();
        let sealed = crypto.seal(b"sk-test-key", b"aad-1").unwrap();
        assert!(crypto.open(&sealed, b"aad-2").is_err());
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(crypto.open(&tampered, b"aad-1").is_err());
        assert!(matches!(
            crypto.open(&[0u8; 5], b"aad-1"),
            Err(CryptoError::InvalidFormat)
        ));
    }

    #[test]
    fn parse_key_accepts_hex_and_base64() {
        let key = [42u8; 32];
        let hex_key = hex::encode(key);
        assert_eq!(parse_key(&hex_key).unwrap(), key);
        let b64 = STANDARD.encode(key);
        assert_eq!(parse_key(&b64).unwrap(), key);
        let b64_url = URL_SAFE_NO_PAD.encode(key);
        assert_eq!(parse_key(&b64_url).unwrap(), key);
        assert!(parse_key("too-short").is_err());
        assert!(parse_key(&"z".repeat(64)).is_err());
    }

    #[test]
    fn key_file_create_and_reload() {
        let dir = std::env::temp_dir().join(format!("gw-master-key-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("master.key");
        let key1 = load_or_create_key_file(&path).unwrap();
        let key2 = load_or_create_key_file(&path).unwrap();
        assert_eq!(key1, key2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        fs::remove_dir_all(dir).ok();
    }
}
