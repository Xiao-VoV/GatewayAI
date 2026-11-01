//! Agent 适配器扩展点。
//!
//! 新增 Agent：实现 `AgentAdapter` 并在 `adapters()` 注册；命令层/UI 不需要改动。

mod aider;
mod claude_code;
mod codex;
pub(crate) mod context;
mod discovery;
mod gemini_cli;
mod passive;

use serde::Serialize;

pub use context::AgentContext;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub config_format: &'static str,
    pub supports_write: bool,
    pub notes: &'static [&'static str],
}

#[derive(Debug, Default)]
pub struct AgentDetection {
    pub installed: bool,
    pub command: Option<String>,
    pub config_path: Option<String>,
    pub config_exists: bool,
    pub endpoint: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub status: String,
    pub command: Option<String>,
    pub config_path: Option<String>,
    pub config_exists: bool,
    pub config_format: String,
    pub supports_write: bool,
    pub endpoint: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentScan {
    pub os: String,
    pub agents: Vec<AgentInfo>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChange {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlan {
    pub agent_id: String,
    pub agent_name: String,
    pub supported: bool,
    pub target_path: Option<String>,
    pub format: String,
    pub changes: Vec<ConfigChange>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentApplyResult {
    pub agent_id: String,
    pub config_path: String,
    pub backup_path: Option<String>,
    pub applied: Vec<ConfigChange>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfigStatus {
    pub applied: bool,
    pub config_path: String,
    pub manifest_path: String,
    pub backup_path: Option<String>,
}

pub struct ConfigRequest {
    pub local_base_url: String,
    pub model: Option<String>,
}

pub trait AgentAdapter: Send + Sync {
    fn descriptor(&self) -> AgentDescriptor;
    fn detect(&self, ctx: &AgentContext) -> AgentDetection;
    fn config_status(&self, ctx: &AgentContext) -> AgentConfigStatus;
    fn plan(&self, ctx: &AgentContext, request: &ConfigRequest) -> Result<AgentPlan, String>;
    fn apply(
        &self,
        ctx: &AgentContext,
        request: &ConfigRequest,
    ) -> Result<AgentApplyResult, String>;
    fn rollback(&self, ctx: &AgentContext) -> Result<(), String>;
}

static ADAPTERS: [&'static dyn AgentAdapter; 9] = [
    &claude_code::CLAUDE_CODE,
    &codex::CODEX,
    &gemini_cli::GEMINI_CLI,
    &aider::AIDER,
    &passive::CONTINUE_DEV,
    &passive::CURSOR,
    &passive::VSCODE,
    &passive::CLINE,
    &passive::ROO_CODE,
];

pub fn adapters() -> &'static [&'static dyn AgentAdapter] {
    &ADAPTERS
}

pub fn find(id: &str) -> Option<&'static dyn AgentAdapter> {
    adapters()
        .iter()
        .copied()
        .find(|adapter| adapter.descriptor().id == id)
}

pub fn scan(ctx: &AgentContext) -> AgentScan {
    let mut agents = Vec::new();
    let mut warnings = Vec::new();
    for adapter in adapters() {
        let descriptor = adapter.descriptor();
        let detection = adapter.detect(ctx);
        let status = if detection.endpoint.is_some() {
            "configured"
        } else if detection.installed || detection.config_exists {
            "available"
        } else {
            "missing"
        };
        let mut notes = descriptor
            .notes
            .iter()
            .map(|note| note.to_string())
            .collect::<Vec<_>>();
        notes.extend(detection.notes);
        agents.push(AgentInfo {
            id: descriptor.id.to_string(),
            name: descriptor.name.to_string(),
            installed: detection.installed,
            status: status.to_string(),
            command: detection.command,
            config_path: detection.config_path,
            config_exists: detection.config_exists,
            config_format: descriptor.config_format.to_string(),
            supports_write: descriptor.supports_write,
            endpoint: detection.endpoint,
            notes,
        });
    }
    if agents.iter().filter(|agent| agent.installed).count() == 0 {
        warnings.push("未检测到已安装的 AI 工具；本机可能尚未安装或安装在非标准路径".to_string());
    }
    AgentScan {
        os: std::env::consts::OS.to_string(),
        agents,
        warnings,
    }
}

pub(crate) fn validate_model_name(model: &str) -> Result<(), String> {
    let valid = model
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '/' | '-'));
    if !valid || model.chars().any(char::is_control) {
        return Err("模型名包含非法字符".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{adapters, find, scan, AgentContext, ConfigRequest};
    use std::path::PathBuf;

    fn test_ctx() -> AgentContext {
        AgentContext::new(
            PathBuf::from("/tmp/gw-agent-test-home"),
            PathBuf::from("/tmp/gw-agent-test-data"),
        )
    }

    #[test]
    fn registry_contains_all_agents_and_ids_are_unique() {
        let mut ids = adapters()
            .iter()
            .map(|adapter| adapter.descriptor().id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), adapters().len());
        assert_eq!(adapters().len(), 9);
        assert!(find("claude_code").is_some());
        assert!(find("not_exists").is_none());
    }

    #[test]
    fn scan_returns_agent_info_without_panicking() {
        let scan = scan(&test_ctx());
        assert_eq!(scan.agents.len(), 9);
        assert!(scan.agents.iter().all(|agent| !agent.name.is_empty()));
    }

    #[test]
    fn passive_agent_plan_is_unsupported() {
        let ctx = test_ctx();
        let adapter = find("cursor").expect("cursor adapter");
        let plan = adapter
            .plan(
                &ctx,
                &ConfigRequest {
                    local_base_url: "http://127.0.0.1:1".to_string(),
                    model: None,
                },
            )
            .expect("plan");
        assert!(!plan.supported);
    }
}
