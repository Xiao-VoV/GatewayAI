use std::fs;
use std::path::PathBuf;

use super::discovery::{cli_detect, toml_line_value};
use super::{
    validate_model_name, AgentAdapter, AgentApplyResult, AgentConfigStatus, AgentContext,
    AgentDescriptor, AgentDetection, AgentPlan, ConfigChange, ConfigRequest,
};
use crate::config_tx;

const PLACEHOLDER: &str = "gw-local-device";

pub(crate) struct Codex;
pub(crate) static CODEX: Codex = Codex;

impl Codex {
    fn config_path(ctx: &AgentContext) -> PathBuf {
        ctx.home_path(".codex/config.toml")
    }

    fn manifest_path(ctx: &AgentContext) -> PathBuf {
        ctx.managed_path("codex.json")
    }
}

impl AgentAdapter for Codex {
    fn descriptor(&self) -> AgentDescriptor {
        AgentDescriptor {
            id: "codex",
            name: "OpenAI Codex CLI",
            config_format: "toml",
            supports_write: true,
            notes: &["Responses 协议；experimental_bearer_token 占据位（无需环境变量）"],
        }
    }

    fn detect(&self, ctx: &AgentContext) -> AgentDetection {
        cli_detect(
            "codex",
            Self::config_path(ctx),
            |text| toml_line_value(text, "base_url"),
            true,
        )
    }

    fn config_status(&self, ctx: &AgentContext) -> AgentConfigStatus {
        let config_path = Self::config_path(ctx);
        let manifest_path = Self::manifest_path(ctx);
        let record: Option<config_tx::Manifest> =
            config_tx::read_manifest(&manifest_path).ok().flatten();
        let applied = fs::read_to_string(&config_path)
            .ok()
            .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
            .and_then(|document| {
                let provider = document.get("model_providers")?.get("gw")?;
                let base_url = provider.get("base_url")?.as_str()?;
                Some(
                    base_url.starts_with("http://127.0.0.1:")
                        || base_url.starts_with("http://localhost:"),
                )
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
                key: "model_provider".to_string(),
                value: "gw".to_string(),
            },
            ConfigChange {
                key: "model_providers.gw.base_url".to_string(),
                value: format!("{}/v1", request.local_base_url),
            },
            ConfigChange {
                key: "model_providers.gw.wire_api".to_string(),
                value: "responses".to_string(),
            },
            ConfigChange {
                key: "model_providers.gw.experimental_bearer_token".to_string(),
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
                value: model.to_string(),
            });
        }
        Ok(AgentPlan {
            agent_id: self.descriptor().id.to_string(),
            agent_name: self.descriptor().name.to_string(),
            supported: true,
            target_path: Some(Self::config_path(ctx).display().to_string()),
            format: "toml".to_string(),
            changes,
            warnings: vec![
                "保留原 TOML 注释/其它表；experimental_bearer_token 为 Codex 实验字段，发布前回归"
                    .to_string(),
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
        let config_path = Self::config_path(ctx);
        let manifest_path = Self::manifest_path(ctx);
        config_tx::ensure_parent(&manifest_path)?;
        let existing = fs::read(&config_path).ok();
        let original = existing
            .as_ref()
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .unwrap_or_default();
        let mut document = original
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("Codex 配置不是合法 TOML：{error}"))?;

        document["model_provider"] = toml_edit::value("gw");
        let mut applied = vec![ConfigChange {
            key: "model_provider".to_string(),
            value: "gw".to_string(),
        }];
        if let Some(model) = model {
            document["model"] = toml_edit::value(model);
            applied.push(ConfigChange {
                key: "model".to_string(),
                value: model.to_string(),
            });
        }
        let providers_item = document
            .entry("model_providers")
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        let providers = providers_item
            .as_table_mut()
            .ok_or_else(|| "Codex 配置 model_providers 必须是 table".to_string())?;
        let provider_item = providers
            .entry("gw")
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        let provider = provider_item
            .as_table_mut()
            .ok_or_else(|| "Codex 配置 model_providers.gw 必须是 table".to_string())?;
        let base_url = format!("{}/v1", request.local_base_url);
        provider["name"] = toml_edit::value("GW Local Router");
        provider["base_url"] = toml_edit::value(&base_url);
        provider["wire_api"] = toml_edit::value("responses");
        provider["experimental_bearer_token"] = toml_edit::value(PLACEHOLDER);
        applied.extend([
            ConfigChange {
                key: "model_providers.gw.name".to_string(),
                value: "GW Local Router".to_string(),
            },
            ConfigChange {
                key: "model_providers.gw.base_url".to_string(),
                value: base_url,
            },
            ConfigChange {
                key: "model_providers.gw.wire_api".to_string(),
                value: "responses".to_string(),
            },
            ConfigChange {
                key: "model_providers.gw.experimental_bearer_token".to_string(),
                value: PLACEHOLDER.to_string(),
            },
        ]);

        let record = config_tx::backup(&config_path, &ctx.backups_dir(), "codex-config")?;
        let manifest = config_tx::build_manifest(record);
        config_tx::write_manifest(&manifest_path, &manifest)?;
        config_tx::write_private(&config_path, document.to_string().as_bytes())?;
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
    use super::{Codex, CODEX};
    use crate::agents::{AgentAdapter, AgentContext, ConfigRequest};
    use std::path::PathBuf;

    fn test_ctx() -> AgentContext {
        let base = std::env::temp_dir().join(format!(
            "gw-codex-test-{}-{}",
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
        let config = Codex::config_path(&ctx);
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        let original = "# My Codex\nmodel = \"old-model\"\napproval_policy = \"never\"\n\n[model_providers.gw]\nname = \"Old\"\n";
        std::fs::write(&config, original).unwrap();
        let result = CODEX
            .apply(&ctx, &request("http://127.0.0.1:6789"))
            .unwrap();
        assert!(result.backup_path.is_some());
        let patched = std::fs::read_to_string(&config).unwrap();
        assert!(patched.contains("# My Codex"));
        assert!(patched.contains("approval_policy = \"never\""));
        assert!(patched.contains("base_url = \"http://127.0.0.1:6789/v1\""));
        assert!(patched.contains("wire_api = \"responses\""));
        assert!(CODEX.config_status(&ctx).applied);
        CODEX.rollback(&ctx).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        std::fs::remove_dir_all(ctx.home).ok();
    }

    #[test]
    fn rejects_bad_toml_and_non_loopback() {
        let ctx = test_ctx();
        let config = Codex::config_path(&ctx);
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, "not = [valid").unwrap();
        assert!(CODEX
            .apply(&ctx, &request("http://127.0.0.1:4567"))
            .is_err());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "not = [valid");
        assert!(CODEX
            .apply(&ctx, &request("https://gw.example.com"))
            .is_err());
        std::fs::remove_dir_all(ctx.home).ok();
    }

    #[allow(dead_code)]
    fn _keep(_: PathBuf) {}
}
