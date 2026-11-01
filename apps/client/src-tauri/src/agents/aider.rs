use std::fs;
use std::path::PathBuf;

use super::discovery::cli_detect;
use super::{
    validate_model_name, AgentAdapter, AgentApplyResult, AgentConfigStatus, AgentContext,
    AgentDescriptor, AgentDetection, AgentPlan, ConfigChange, ConfigRequest,
};
use crate::config_tx;

const PLACEHOLDER: &str = "gw-local-device";

pub(crate) struct Aider;
pub(crate) static AIDER: Aider = Aider;

impl Aider {
    fn config_path(ctx: &AgentContext) -> PathBuf {
        ctx.home_path(".aider.conf.yml")
    }

    fn manifest_path(ctx: &AgentContext) -> PathBuf {
        ctx.managed_path("aider.json")
    }
}

fn endpoint_from_config(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix("openai-api-base:")?.trim();
        if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        }
    })
}

impl AgentAdapter for Aider {
    fn descriptor(&self) -> AgentDescriptor {
        AgentDescriptor {
            id: "aider",
            name: "Aider",
            config_format: "yaml",
            supports_write: true,
            notes: &["OpenAI Chat 兼容；行级 upsert 保留其它配置/注释"],
        }
    }

    fn detect(&self, ctx: &AgentContext) -> AgentDetection {
        cli_detect("aider", Self::config_path(ctx), endpoint_from_config, true)
    }

    fn config_status(&self, ctx: &AgentContext) -> AgentConfigStatus {
        let config_path = Self::config_path(ctx);
        let manifest_path = Self::manifest_path(ctx);
        let record: Option<config_tx::Manifest> =
            config_tx::read_manifest(&manifest_path).ok().flatten();
        let applied = fs::read_to_string(&config_path)
            .ok()
            .and_then(|text| endpoint_from_config(&text))
            .map(|value| {
                value.starts_with("http://127.0.0.1:") || value.starts_with("http://localhost:")
            })
            .unwrap_or(false);
        AgentConfigStatus {
            applied,
            config_path: config_path.display().to_string(),
            manifest_path: manifest_path.display().to_string(),
            backup_path: record.and_then(|record| record.backup_path),
        }
    }

    fn plan(&self, ctx: &AgentContext, request: &ConfigRequest) -> Result<AgentPlan, String> {
        config_tx::validate_local_base_url(&request.local_base_url)?;
        let mut changes = vec![
            ConfigChange {
                key: "openai-api-base".to_string(),
                value: format!("{}/v1", request.local_base_url),
            },
            ConfigChange {
                key: "openai-api-key".to_string(),
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
                key: "model".to_string(),
                value: format!("openai/{model}"),
            });
        }
        Ok(AgentPlan {
            agent_id: self.descriptor().id.to_string(),
            agent_name: self.descriptor().name.to_string(),
            supported: true,
            target_path: Some(Self::config_path(ctx).display().to_string()),
            format: "yaml".to_string(),
            changes,
            warnings: vec!["只改用户级 ~/.aider.conf.yml 顶层键；保留其它行".to_string()],
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
        let config_path = Self::config_path(ctx);
        let manifest_path = Self::manifest_path(ctx);
        config_tx::ensure_parent(&manifest_path)?;
        let existing = fs::read(&config_path).ok();
        let original = existing
            .as_ref()
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .unwrap_or_default();
        let mut patch = config_tx::upsert_yaml_scalar(
            &original,
            "openai-api-base",
            &format!("{}/v1", request.local_base_url),
        );
        patch = config_tx::upsert_yaml_scalar(&patch, "openai-api-key", PLACEHOLDER);
        let mut applied = vec![
            ConfigChange {
                key: "openai-api-base".to_string(),
                value: format!("{}/v1", request.local_base_url),
            },
            ConfigChange {
                key: "openai-api-key".to_string(),
                value: PLACEHOLDER.to_string(),
            },
        ];
        if let Some(model) = model {
            let model_value = if model.starts_with("openai/") {
                model.to_string()
            } else {
                format!("openai/{model}")
            };
            patch = config_tx::upsert_yaml_scalar(&patch, "model", &model_value);
            applied.push(ConfigChange {
                key: "model".to_string(),
                value: model_value,
            });
        }
        let record = config_tx::backup(&config_path, &ctx.backups_dir(), "aider-conf")?;
        let manifest = config_tx::build_manifest(record);
        config_tx::write_manifest(&manifest_path, &manifest)?;
        config_tx::write_private(&config_path, patch.as_bytes())?;
        Ok(AgentApplyResult {
            agent_id: self.descriptor().id.to_string(),
            config_path: config_path.display().to_string(),
            backup_path: manifest.backup_path,
            applied,
        })
    }

    fn rollback(&self, ctx: &AgentContext) -> Result<(), String> {
        config_tx::rollback_single(&Self::config_path(ctx), &Self::manifest_path(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::{Aider, AIDER};
    use crate::agents::{AgentAdapter, AgentContext, ConfigRequest};
    use std::path::PathBuf;

    fn test_ctx() -> AgentContext {
        let base = std::env::temp_dir().join(format!(
            "gw-aider-test-{}-{}",
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
    fn apply_rollback_preserves_lines() {
        let ctx = test_ctx();
        let config = Aider::config_path(&ctx);
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        let original = "# Aider config\nmodel: openai/old\nopenai-api-base: http://example.com/v1\nother: keep\n";
        std::fs::write(&config, original).unwrap();
        AIDER
            .apply(
                &ctx,
                &ConfigRequest {
                    local_base_url: "http://127.0.0.1:5678".to_string(),
                    model: Some("claude-sonnet".to_string()),
                },
            )
            .unwrap();
        let patched = std::fs::read_to_string(&config).unwrap();
        assert!(patched.contains("# Aider config"));
        assert!(patched.contains("other: keep"));
        assert!(patched.contains("openai-api-base: http://127.0.0.1:5678/v1"));
        assert!(patched.contains("model: openai/claude-sonnet"));
        assert!(AIDER.config_status(&ctx).applied);
        AIDER.rollback(&ctx).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        std::fs::remove_dir_all(ctx.home).ok();
    }

    #[test]
    fn rejects_non_loopback_and_bad_model() {
        let ctx = test_ctx();
        assert!(AIDER
            .apply(
                &ctx,
                &ConfigRequest {
                    local_base_url: "https://gw.example.com".to_string(),
                    model: None,
                },
            )
            .is_err());
        assert!(AIDER
            .apply(
                &ctx,
                &ConfigRequest {
                    local_base_url: "http://127.0.0.1:4567".to_string(),
                    model: Some("bad\nmodel".to_string()),
                },
            )
            .is_err());
        std::fs::remove_dir_all(ctx.home).ok();
    }

    #[allow(dead_code)]
    fn _keep(_: PathBuf) {}
}
