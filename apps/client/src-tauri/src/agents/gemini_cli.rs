use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::discovery::{match_config_value, read_small, which};
use super::{
    validate_model_name, AgentAdapter, AgentApplyResult, AgentConfigStatus, AgentContext,
    AgentDescriptor, AgentDetection, AgentPlan, ConfigChange, ConfigRequest,
};
use crate::config_tx::{self, FileRecord};

const PLACEHOLDER: &str = "gw-local-device";

pub(crate) struct GeminiCli;
pub(crate) static GEMINI_CLI: GeminiCli = GeminiCli;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiManifest {
    settings: FileRecord,
    env_file: FileRecord,
    applied_at: u64,
}

impl GeminiCli {
    fn settings_path(ctx: &AgentContext) -> PathBuf {
        ctx.home_path(".gemini/settings.json")
    }

    fn env_path(ctx: &AgentContext) -> PathBuf {
        ctx.home_path(".gemini/.env")
    }

    fn manifest_path(ctx: &AgentContext) -> PathBuf {
        ctx.managed_path("gemini.json")
    }
}

fn endpoint_from_env(text: &str) -> Option<String> {
    match_config_value(text, "GOOGLE_GEMINI_BASE_URL")
}

impl AgentAdapter for GeminiCli {
    fn descriptor(&self) -> AgentDescriptor {
        AgentDescriptor {
            id: "gemini_cli",
            name: "Gemini CLI",
            config_format: "json+dotenv",
            supports_write: true,
            notes: &["GOOGLE_GEMINI_BASE_URL 对 localhost 允许 http；走 /v1beta 原生协议"],
        }
    }

    fn detect(&self, ctx: &AgentContext) -> AgentDetection {
        let command_path = which("gemini");
        let settings = Self::settings_path(ctx);
        let env = Self::env_path(ctx);
        let config_exists = settings.exists() || env.exists();
        let endpoint = read_small(&env).and_then(|text| endpoint_from_env(&text));
        let installed = command_path.is_some() || config_exists;
        let mut notes = Vec::new();
        if config_exists && endpoint.is_none() {
            notes.push("配置文件存在，但未检测到 GOOGLE_GEMINI_BASE_URL".to_string());
        }
        AgentDetection {
            installed,
            command: command_path.map(|path| path.display().to_string()),
            config_path: Some(settings.display().to_string()),
            config_exists,
            endpoint,
            notes,
        }
    }

    fn config_status(&self, ctx: &AgentContext) -> AgentConfigStatus {
        let settings = Self::settings_path(ctx);
        let env = Self::env_path(ctx);
        let manifest_path = Self::manifest_path(ctx);
        let manifest: Option<GeminiManifest> =
            config_tx::read_manifest(&manifest_path).ok().flatten();
        let auth_ok = fs::read(&settings)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| {
                value
                    .get("security")?
                    .get("auth")?
                    .get("selectedType")?
                    .as_str()
                    .map(|value| value == "gemini-api-key")
            })
            .unwrap_or(false);
        let env_ok = fs::read_to_string(&env)
            .ok()
            .and_then(|text| endpoint_from_env(&text))
            .map(|value| {
                value.starts_with("http://127.0.0.1:") || value.starts_with("http://localhost:")
            })
            .unwrap_or(false);
        AgentConfigStatus {
            applied: auth_ok && env_ok,
            config_path: settings.display().to_string(),
            manifest_path: manifest_path.display().to_string(),
            backup_path: manifest.and_then(|manifest| manifest.settings.backup_path),
        }
    }

    fn plan(&self, ctx: &AgentContext, request: &ConfigRequest) -> Result<AgentPlan, String> {
        config_tx::validate_local_base_url(&request.local_base_url)?;
        let mut changes = vec![
            ConfigChange {
                key: "security.auth.selectedType".to_string(),
                value: "gemini-api-key".to_string(),
            },
            ConfigChange {
                key: "GOOGLE_GEMINI_BASE_URL".to_string(),
                value: request.local_base_url.clone(),
            },
            ConfigChange {
                key: "GEMINI_API_KEY".to_string(),
                value: PLACEHOLDER.to_string(),
            },
        ];
        if let Some(model) = request
            .model
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            validate_model_name(model)?;
            changes.push(ConfigChange {
                key: "GEMINI_MODEL".to_string(),
                value: model.to_string(),
            });
        }
        Ok(AgentPlan {
            agent_id: self.descriptor().id.to_string(),
            agent_name: self.descriptor().name.to_string(),
            supported: true,
            target_path: Some(Self::settings_path(ctx).display().to_string()),
            format: "json+dotenv".to_string(),
            changes,
            warnings: vec!["会切换 auth 为 API key；双文件备份，回滚恢复原 auth".to_string()],
        })
    }

    fn apply(
        &self,
        ctx: &AgentContext,
        request: &ConfigRequest,
    ) -> Result<AgentApplyResult, String> {
        config_tx::validate_local_base_url(&request.local_base_url)?;
        let model = request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(model) = model {
            validate_model_name(model)?;
        }
        let settings_path = Self::settings_path(ctx);
        let env_path = Self::env_path(ctx);
        let manifest_path = Self::manifest_path(ctx);
        config_tx::ensure_parent(&manifest_path)?;

        let existing_settings = fs::read(&settings_path).ok();
        let mut settings: Value = match &existing_settings {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|error| format!("Gemini settings.json 不是合法 JSON：{error}"))?,
            None => json!({}),
        };
        let Some(root) = settings.as_object_mut() else {
            return Err("Gemini settings.json 根节点必须是对象".to_string());
        };
        let security = root.entry("security").or_insert_with(|| json!({}));
        let Some(security) = security.as_object_mut() else {
            return Err("Gemini settings.json security 必须是对象".to_string());
        };
        let auth = security.entry("auth").or_insert_with(|| json!({}));
        let Some(auth) = auth.as_object_mut() else {
            return Err("Gemini settings.json security.auth 必须是对象".to_string());
        };
        auth.insert(
            "selectedType".to_string(),
            Value::String("gemini-api-key".to_string()),
        );

        let existing_env = fs::read(&env_path).ok();
        let env_text = existing_env
            .as_ref()
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .unwrap_or_default();
        let mut env = config_tx::upsert_dotenv_value(
            &env_text,
            "GOOGLE_GEMINI_BASE_URL",
            &request.local_base_url,
        );
        env = config_tx::upsert_dotenv_value(&env, "GEMINI_API_KEY", PLACEHOLDER);
        let mut applied = vec![
            ConfigChange {
                key: "security.auth.selectedType".to_string(),
                value: "gemini-api-key".to_string(),
            },
            ConfigChange {
                key: "GOOGLE_GEMINI_BASE_URL".to_string(),
                value: request.local_base_url.clone(),
            },
            ConfigChange {
                key: "GEMINI_API_KEY".to_string(),
                value: PLACEHOLDER.to_string(),
            },
        ];
        if let Some(model) = model {
            env = config_tx::upsert_dotenv_value(&env, "GEMINI_MODEL", model);
            applied.push(ConfigChange {
                key: "GEMINI_MODEL".to_string(),
                value: model.to_string(),
            });
        }

        let manifest = GeminiManifest {
            settings: config_tx::backup(&settings_path, &ctx.backups_dir(), "gemini-settings")?,
            env_file: config_tx::backup(&env_path, &ctx.backups_dir(), "gemini-env")?,
            applied_at: config_tx::unix_secs(),
        };
        config_tx::write_manifest(&manifest_path, &manifest)?;
        let settings_bytes = serde_json::to_vec_pretty(&settings)
            .map_err(|error| format!("序列化 settings 失败：{error}"))?;
        config_tx::write_private(&settings_path, &settings_bytes)?;
        config_tx::write_private(&env_path, env.as_bytes())?;
        Ok(AgentApplyResult {
            agent_id: self.descriptor().id.to_string(),
            config_path: settings_path.display().to_string(),
            backup_path: manifest.settings.backup_path,
            applied,
        })
    }

    fn rollback(&self, ctx: &AgentContext) -> Result<(), String> {
        let manifest_path = Self::manifest_path(ctx);
        let Some(manifest): Option<GeminiManifest> = config_tx::read_manifest(&manifest_path)?
        else {
            return Err("没有找到由本客户端写入的 Gemini 配置".to_string());
        };
        config_tx::restore_file(&Self::settings_path(ctx), &manifest.settings)?;
        config_tx::restore_file(&Self::env_path(ctx), &manifest.env_file)?;
        match fs::remove_file(&manifest_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("清理 manifest 失败：{error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GeminiCli, GEMINI_CLI};
    use crate::agents::{AgentAdapter, AgentContext, ConfigRequest};
    use std::path::PathBuf;

    fn test_ctx() -> AgentContext {
        let base = std::env::temp_dir().join(format!(
            "gw-gemini-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        AgentContext::new(base.clone(), base.join("appdata"))
    }

    #[test]
    fn two_file_transaction_and_rollback() {
        let ctx = test_ctx();
        let settings = GeminiCli::settings_path(&ctx);
        let env = GeminiCli::env_path(&ctx);
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original_settings =
            r#"{"theme":"dark","security":{"auth":{"selectedType":"oauth-personal"}}}"#;
        let original_env = "# gemini env\nOTHER=keep\n";
        std::fs::write(&settings, original_settings).unwrap();
        std::fs::write(&env, original_env).unwrap();
        GEMINI_CLI
            .apply(
                &ctx,
                &ConfigRequest {
                    local_base_url: "http://127.0.0.1:7890".to_string(),
                    model: Some("claude-sonnet".to_string()),
                },
            )
            .unwrap();
        let patched_env = std::fs::read_to_string(&env).unwrap();
        assert!(patched_env.contains("# gemini env"));
        assert!(patched_env.contains("OTHER=keep"));
        assert!(patched_env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:7890"));
        let patched_settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        assert_eq!(
            patched_settings["security"]["auth"]["selectedType"],
            "gemini-api-key"
        );
        assert_eq!(patched_settings["theme"], "dark");
        assert!(GEMINI_CLI.config_status(&ctx).applied);
        GEMINI_CLI.rollback(&ctx).unwrap();
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            original_settings
        );
        assert_eq!(std::fs::read_to_string(&env).unwrap(), original_env);
        std::fs::remove_dir_all(ctx.home).ok();
    }

    #[test]
    fn rejects_invalid_settings_and_non_loopback() {
        let ctx = test_ctx();
        let settings = GeminiCli::settings_path(&ctx);
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{not json").unwrap();
        assert!(GEMINI_CLI
            .apply(
                &ctx,
                &ConfigRequest {
                    local_base_url: "http://127.0.0.1:7890".to_string(),
                    model: None,
                },
            )
            .is_err());
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{not json");
        std::fs::write(&settings, "{}").unwrap();
        assert!(GEMINI_CLI
            .apply(
                &ctx,
                &ConfigRequest {
                    local_base_url: "https://gw.example.com".to_string(),
                    model: None,
                },
            )
            .is_err());
        std::fs::remove_dir_all(ctx.home).ok();
    }

    #[allow(dead_code)]
    fn _keep(_: PathBuf) {}
}
