//! 通用配置事务：0600 文件、备份、manifest、精确回滚。
//!
//! Agent 适配器只声明文件与内容；权限/备份/回滚语义集中在此，避免每个 Agent 重复。

use std::fs;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub backup_path: Option<String>,
    pub file_existed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub backup_path: Option<String>,
    pub file_existed: bool,
    pub applied_at: u64,
}

pub fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    ensure_parent(path)?;
    fs::write(path, bytes).map_err(|error| format!("写入失败 {}: {error}", path.display()))?;
    set_private_permissions(path)
}

pub fn validate_local_base_url(local_base_url: &str) -> Result<(), String> {
    let valid = local_base_url.starts_with("http://127.0.0.1:")
        || local_base_url.starts_with("http://localhost:");
    if !valid {
        return Err("配置写入只允许本地路由地址（127.0.0.1/localhost）".to_string());
    }
    if local_base_url.chars().any(char::is_whitespace)
        || local_base_url.chars().any(char::is_control)
    {
        return Err("本地路由地址包含非法字符".to_string());
    }
    Ok(())
}

pub fn backup(path: &Path, backup_dir: &Path, prefix: &str) -> Result<FileRecord, String> {
    fs::create_dir_all(backup_dir).map_err(|error| format!("创建备份目录失败：{error}"))?;
    match fs::read(path) {
        Ok(bytes) => {
            let target = backup_dir.join(format!("{prefix}-{}.bak", unix_secs()));
            write_private(&target, &bytes)?;
            Ok(FileRecord {
                backup_path: Some(target.display().to_string()),
                file_existed: true,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileRecord {
            backup_path: None,
            file_existed: false,
        }),
        Err(error) => Err(format!("读取配置失败 {}: {error}", path.display())),
    }
}

pub fn write_manifest<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    write_private(path, &bytes)
}

pub fn read_manifest<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("manifest 解析失败：{error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取 manifest 失败：{error}")),
    }
}

pub fn restore_file(path: &Path, record: &FileRecord) -> Result<(), String> {
    if record.file_existed {
        let backup = record
            .backup_path
            .as_ref()
            .ok_or_else(|| "manifest 缺少备份路径".to_string())?;
        let bytes = fs::read(backup).map_err(|error| format!("读取备份失败：{error}"))?;
        write_private(path, &bytes)
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("删除已创建配置失败：{error}")),
        }
    }
}

pub fn rollback_single(config_path: &Path, manifest_path: &Path) -> Result<(), String> {
    let Some(manifest): Option<Manifest> = read_manifest(manifest_path)? else {
        return Err("没有找到由本客户端写入的配置".to_string());
    };
    restore_file(
        config_path,
        &FileRecord {
            backup_path: manifest.backup_path,
            file_existed: manifest.file_existed,
        },
    )?;
    match fs::remove_file(manifest_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清理 manifest 失败：{error}")),
    }
}

pub fn build_manifest(record: FileRecord) -> Manifest {
    Manifest {
        backup_path: record.backup_path,
        file_existed: record.file_existed,
        applied_at: unix_secs(),
    }
}

/// 行级 YAML 顶层标量 upsert：保留注释/其它键。
pub fn upsert_yaml_scalar(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&format!("{key}:")) {
            let indent = &line[..line.len() - trimmed.len()];
            lines.push(format!("{indent}{key}: {value}"));
            found = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !found {
        lines.push(format!("{key}: {value}"));
    }
    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// dotenv 顶层 KEY=VALUE upsert：保留注释/其它键。
pub fn upsert_dotenv_value(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&format!("{key}=")) {
            lines.push(format!("{key}={value}"));
            found = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !found {
        lines.push(format!("{key}={value}"));
    }
    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{upsert_dotenv_value, upsert_yaml_scalar, validate_local_base_url};

    #[test]
    fn yaml_upsert_preserves_other_lines() {
        let text = "# c\nkeep: 1\nopenai-api-base: old\n";
        let out = upsert_yaml_scalar(text, "openai-api-base", "http://127.0.0.1:1/v1");
        assert!(out.contains("# c"));
        assert!(out.contains("keep: 1"));
        assert!(out.contains("openai-api-base: http://127.0.0.1:1/v1"));
        assert_eq!(out.matches("openai-api-base:").count(), 1);
    }

    #[test]
    fn dotenv_upsert_preserves_other_lines() {
        let text = "# env\nKEEP=1\n";
        let out = upsert_dotenv_value(text, "GEMINI_API_KEY", "x");
        assert!(out.contains("# env"));
        assert!(out.contains("KEEP=1"));
        assert!(out.contains("GEMINI_API_KEY=x"));
    }

    #[test]
    fn loopback_only() {
        assert!(validate_local_base_url("http://127.0.0.1:1").is_ok());
        assert!(validate_local_base_url("http://localhost:1").is_ok());
        assert!(validate_local_base_url("https://gw.example.com").is_err());
    }
}
