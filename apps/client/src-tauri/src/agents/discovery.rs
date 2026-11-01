use std::path::{Path, PathBuf};

use super::AgentDetection;

pub fn which(command: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(command);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{command}.exe"));
            if is_executable(&exe) {
                return Some(exe);
            }
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn read_small(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

pub fn json_string_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = text.find(&needle)?;
    let rest = &text[start + needle.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub fn toml_line_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(key)?;
        let rest = rest.trim_start().strip_prefix('=')?.trim();
        let value = rest.trim_matches('"').to_string();
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    })
}

pub fn match_config_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(&format!("{key}="))?.trim();
        if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        }
    })
}

pub fn app_path(mac: &[&str], linux: &[&str], windows: &[&str]) -> Option<PathBuf> {
    let candidates: Vec<&str> = if cfg!(target_os = "macos") {
        mac.to_vec()
    } else if cfg!(target_os = "windows") {
        windows.to_vec()
    } else {
        linux.to_vec()
    };
    let home = super::context::home_dir();
    for candidate in candidates {
        if let Some(stripped) = candidate.strip_prefix("~/") {
            if let Some(home) = &home {
                let path = home.join(stripped);
                if path.exists() {
                    return Some(path);
                }
                continue;
            }
        }
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

pub fn cli_detect(
    command: &str,
    config_path: PathBuf,
    detect_endpoint: impl Fn(&str) -> Option<String>,
    config_path_label: bool,
) -> AgentDetection {
    let command_path = which(command);
    let config_exists = config_path.exists();
    let endpoint = read_small(&config_path)
        .as_deref()
        .and_then(detect_endpoint);
    let installed = command_path.is_some() || config_exists;
    let mut notes = Vec::new();
    if config_exists && endpoint.is_none() {
        notes.push("配置文件存在，但未检测到自定义网关端点".to_string());
    }
    if endpoint.is_some() {
        notes.push("检测到已有自定义端点；写入前需确认是否为企业配置".to_string());
    }
    AgentDetection {
        installed,
        command: command_path.map(|path| path.display().to_string()),
        config_path: config_path_label.then(|| config_path.display().to_string()),
        config_exists,
        endpoint,
        notes,
    }
}
