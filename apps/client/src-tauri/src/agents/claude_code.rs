use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::discovery::{cli_detect, json_string_field};
use super::{
    validate_model_name, AgentAdapter, AgentApplyResult, AgentConfigStatus, AgentContext,
    AgentDescriptor, AgentDetection, AgentPlan, ConfigChange, ConfigRequest,
};
use crate::config_tx;

const PLACEHOLDER: &str = "gw-local-device";

pub(crate) struct ClaudeCode;
pub(crate) static CLAUDE_CODE: ClaudeCode = ClaudeCode;

impl ClaudeCode {
    fn settings_path(ctx: &AgentContext) -> PathBuf {
        ctx.home_path(".claude/settings.json")
    }

    fn manifest_path(ctx: &AgentContext) -> PathBuf {
        ctx.managed_path("claude.json")
    }
}

impl AgentAdapter for ClaudeCode {
    fn descriptor(&self) -> AgentDescriptor {
        AgentDescriptor {
            id: "claude_code",
            name: "Claude Code",
            config_format: "json",
            supports_write: true,
            notes: &["官方变量 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN"],
        }
    }

    fn detect(&self, ctx: &AgentContext) -> AgentDetection {
        cli_detect(
            "claude",
            Self::settings_path(ctx),
            |text| json_string_field(text, "ANTHROPIC_BASE_URL"),
            true,
        )
    }

    fn config_status(&self, ctx: &AgentContext) -> AgentConfigStatus {
        let settings = Self::settings_path(ctx);
        let manifest = Self::manifest_path(ctx);
        let record: Option<config_tx::Manifest> =
            config_tx::read_manifest(&manifest).ok().flatten();
        let applied = fs::read(&settings)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| {
                value
                    .get("env")?
                    .get("ANTHROPIC_BASE_URL")?
                    .as_str()
                    .map(|value| {
                        value.starts_with("http://127.0.0.1:")
                            || value.starts_with("http://localhost:")
                    })
            })
            .unwrap_or(false);
        AgentConfigStatus {
            applied,
            config_path: settings.display().to_string(),
            manifest_path: manifest.display().to_string(),
            backup_path: record.and_then(|record| record.backup_path),
        }
    }

    fn plan(&self, ctx: &AgentContext, request: &ConfigRequest) -> Result<AgentPlan, String> {
        config_tx::validate_local_base_url(&request.local_base_url)?;
        let mut changes = vec![
            ConfigChange {
                key: "env.ANTHROPIC_BASE_URL".to_string(),
                value: request.local_base_url.clone(),
            },
            ConfigChange {
                key: "env.ANTHROPIC_AUTH_TOKEN".to_string(),
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
                key: "env.ANTHROPIC_MODEL".to_string(),
                value: model.to_string(),
            });
        }
        Ok(AgentPlan {
            agent_id: self.descriptor().id.to_string(),
            agent_name: self.descriptor().name.to_string(),
            supported: true,
            target_path: Some(Self::settings_path(ctx).display().to_string()),
            format: "json".to_string(),
            changes,
            warnings: vec![
                "只写用户级 settings.json 的 env；不碰 managed settings/shell profile".to_string(),
            ],
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
        let manifest_path = Self::manifest_path(ctx);
        config_tx::ensure_parent(&manifest_path)?;

        let existing = fs::read(&settings_path).ok();
        let mut root: Value = match &existing {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|error| format!("Claude Code 配置不是合法 JSON：{error}"))?,
            None => json!({}),
        };
        let Some(root_object) = root.as_object_mut() else {
            return Err("Claude Code 配置根节点必须是 JSON 对象".to_string());
        };
        let env_value = root_object.entry("env").or_insert_with(|| json!({}));
        let Some(env) = env_value.as_object_mut() else {
            return Err("Claude Code 配置 env 必须是 JSON 对象".to_string());
        };

        let record = config_tx::backup(&settings_path, &ctx.backups_dir(), "claude-settings")?;
        let manifest = config_tx::build_manifest(record);
        config_tx::write_manifest(&manifest_path, &manifest)?;

        env.insert(
            "ANTHROPIC_BASE_URL".to_string(),
            Value::String(request.local_base_url.clone()),
        );
        env.insert(
            "ANTHROPIC_AUTH_TOKEN".to_string(),
            Value::String(PLACEHOLDER.to_string()),
        );
        let mut applied = vec![
            ConfigChange {
                key: "env.ANTHROPIC_BASE_URL".to_string(),
                value: request.local_base_url.clone(),
            },
            ConfigChange {
                key: "env.ANTHROPIC_AUTH_TOKEN".to_string(),
                value: PLACEHOLDER.to_string(),
            },
        ];
        if let Some(model) = model {
            env.insert(
                "ANTHROPIC_MODEL".to_string(),
                Value::String(model.to_string()),
            );
            applied.push(ConfigChange {
                key: "env.ANTHROPIC_MODEL".to_string(),
                value: model.to_string(),
            });
        }
        let bytes =
            serde_json::to_vec_pretty(&root).map_err(|error| format!("序列化配置失败：{error}"))?;
        config_tx::write_private(&settings_path, &bytes)?;
        Ok(AgentApplyResult {
            agent_id: self.descriptor().id.to_string(),
            config_path: settings_path.display().to_string(),
            backup_path: manifest.backup_path,
            applied,
        })
    }

    fn rollback(&self, ctx: &AgentContext) -> Result<(), String> {
        config_tx::rollback_single(&Self::settings_path(ctx), &Self::manifest_path(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::{ClaudeCode, CLAUDE_CODE};
    use crate::agents::{AgentAdapter, AgentContext, ConfigRequest};
    use std::path::PathBuf;

    fn test_ctx() -> AgentContext {
        let base = std::env::temp_dir().join(format!(
            "gw-claude-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        AgentContext::new(base.clone(), base.join("appdata"))
    }

    fn request(url: &str) -> ConfigRequest {
        ConfigRequest {
            local_base_url: url.to_string(),
            model: Some("claude-sonnet".to_string()),
        }
    }

    #[test]
    fn apply_rollback_and_status() {
        let ctx = test_ctx();
        let settings = ClaudeCode::settings_path(&ctx);
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original = r#"{"env":{"KEEP":"yes"},"permissions":{"allow":["Bash"]}}"#;
        std::fs::write(&settings, original).unwrap();
        let result = CLAUDE_CODE
            .apply(&ctx, &request("http://127.0.0.1:4567"))
            .unwrap();
        assert!(result.backup_path.is_some());
        assert!(CLAUDE_CODE.config_status(&ctx).applied);
        let patched: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        assert_eq!(patched["env"]["KEEP"], "yes");
        assert_eq!(
            patched["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:4567"
        );
        CLAUDE_CODE.rollback(&ctx).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
        assert!(!CLAUDE_CODE.config_status(&ctx).applied);
        std::fs::remove_dir_all(ctx.home).ok();
    }

    #[test]
    fn rejects_non_loopback() {
        let ctx = test_ctx();
        assert!(CLAUDE_CODE
            .apply(&ctx, &request("https://gw.example.com"))
            .is_err());
        std::fs::remove_dir_all(ctx.home).ok();
    }

    #[test]
    fn rollback_removes_created_file() {
        let ctx = test_ctx();
        CLAUDE_CODE
            .apply(&ctx, &request("http://127.0.0.1:4567"))
            .unwrap();
        assert!(ClaudeCode::settings_path(&ctx).exists());
        CLAUDE_CODE.rollback(&ctx).unwrap();
        assert!(!ClaudeCode::settings_path(&ctx).exists());
        assert!(CLAUDE_CODE.rollback(&ctx).is_err());
        std::fs::remove_dir_all(ctx.home).ok();
    }

    // 保持可见性，避免 unused import 在 cfg(test) 下报警。
    #[allow(dead_code)]
    fn _keep(_: PathBuf) {}
}
