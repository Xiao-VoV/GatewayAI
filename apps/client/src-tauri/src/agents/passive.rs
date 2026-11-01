//! 只读检测型 Agent：暂无安全自动写入方案，先提供发现/指引。
//! 后续若要写配置，新增独立 adapter 并注册，不要在这里堆分支。

use std::path::PathBuf;

use super::discovery::{app_path, read_small};
use super::{
    AgentAdapter, AgentApplyResult, AgentConfigStatus, AgentContext, AgentDescriptor,
    AgentDetection, AgentPlan, ConfigRequest,
};

pub(crate) struct PassiveAgent {
    descriptor: AgentDescriptor,
    detect: fn(&AgentContext) -> AgentDetection,
}

impl PassiveAgent {
    fn unsupported_plan(&self, _ctx: &AgentContext, target: Option<PathBuf>) -> AgentPlan {
        AgentPlan {
            agent_id: self.descriptor.id.to_string(),
            agent_name: self.descriptor.name.to_string(),
            supported: false,
            target_path: target.map(|path| path.display().to_string()),
            format: self.descriptor.config_format.to_string(),
            changes: Vec::new(),
            warnings: self
                .descriptor
                .notes
                .iter()
                .map(|note| note.to_string())
                .chain(std::iter::once(
                    "该 Agent 当前仅支持检测/配置预览；自动写入需实现新的 AgentAdapter 并评审"
                        .to_string(),
                ))
                .collect(),
        }
    }
}

impl AgentAdapter for PassiveAgent {
    fn descriptor(&self) -> AgentDescriptor {
        self.descriptor.clone()
    }

    fn detect(&self, ctx: &AgentContext) -> AgentDetection {
        (self.detect)(ctx)
    }

    fn config_status(&self, ctx: &AgentContext) -> AgentConfigStatus {
        let detection = self.detect(ctx);
        AgentConfigStatus {
            applied: false,
            config_path: detection.config_path.unwrap_or_default(),
            manifest_path: String::new(),
            backup_path: None,
        }
    }

    fn plan(&self, ctx: &AgentContext, _request: &ConfigRequest) -> Result<AgentPlan, String> {
        let detection = self.detect(ctx);
        Ok(self.unsupported_plan(ctx, detection.config_path.map(PathBuf::from)))
    }

    fn apply(
        &self,
        _ctx: &AgentContext,
        _request: &ConfigRequest,
    ) -> Result<AgentApplyResult, String> {
        Err(format!("{} 暂不支持自动写入配置", self.descriptor.name))
    }

    fn rollback(&self, _ctx: &AgentContext) -> Result<(), String> {
        Err(format!("{} 没有由本客户端写入的配置", self.descriptor.name))
    }
}

fn path_detection(
    installed: bool,
    config_path: PathBuf,
    endpoint: Option<String>,
    notes: Vec<String>,
) -> AgentDetection {
    AgentDetection {
        installed: installed || config_path.exists(),
        command: None,
        config_path: Some(config_path.display().to_string()),
        config_exists: config_path.exists(),
        endpoint,
        notes,
    }
}

fn detect_continue(ctx: &AgentContext) -> AgentDetection {
    let path = ctx.home_path(".continue/config.yaml");
    let endpoint = read_small(&path).and_then(|text| {
        text.lines().find_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("apiBase:")?.trim();
            if rest.is_empty() {
                None
            } else {
                Some(rest.trim_matches('"').to_string())
            }
        })
    });
    path_detection(
        false,
        path,
        endpoint,
        vec!["配置文件可写，但当前仅扫描；Hub 同步可能覆盖".to_string()],
    )
}

fn detect_cursor(ctx: &AgentContext) -> AgentDetection {
    let app = app_path(
        &["/Applications/Cursor.app", "~/Applications/Cursor.app"],
        &["/usr/bin/cursor"],
        &["%LOCALAPPDATA%\\Programs\\Cursor\\Cursor.exe"],
    );
    let config = ctx.home_path("Library/Application Support/Cursor/User/settings.json");
    path_detection(
        app.is_some(),
        config,
        None,
        vec!["仅 BYOK Override 通道可接管；Tab/自营模型需系统代理".to_string()],
    )
}

fn detect_vscode(ctx: &AgentContext) -> AgentDetection {
    let app = app_path(
        &["/Applications/Visual Studio Code.app"],
        &["/usr/bin/code"],
        &["%LOCALAPPDATA%\\Programs\\Microsoft VS Code\\Code.exe"],
    );
    let config = ctx.home_path("Library/Application Support/Code/User/settings.json");
    path_detection(
        app.is_some(),
        config,
        None,
        vec!["BYOK Custom Endpoint 仅覆盖 Chat；补全/检索走 Copilot".to_string()],
    )
}

fn detect_storage(ctx: &AgentContext, storage_name: &str, note: &str) -> AgentDetection {
    let dir = ctx
        .home_path("Library/Application Support/Code/User/globalStorage")
        .join(storage_name);
    path_detection(dir.exists(), dir, None, vec![note.to_string()])
}

fn detect_cline(ctx: &AgentContext) -> AgentDetection {
    detect_storage(
        ctx,
        "saoudrizwan.claude-dev",
        "VS Code 扩展 globalStorage；结构随版本变化",
    )
}

fn detect_roo(ctx: &AgentContext) -> AgentDetection {
    detect_storage(
        ctx,
        "rooveterinaryinc.roo-cline",
        "VS Code 扩展 globalStorage；结构随版本变化",
    )
}

// 只读检测型 Agent；id/name/notes 由 Rust 提供，UI 不需要硬编码。
pub(crate) static CONTINUE_DEV: PassiveAgent = PassiveAgent {
    descriptor: AgentDescriptor {
        id: "continue_dev",
        name: "Continue.dev",
        config_format: "yaml",
        supports_write: false,
        notes: &["~/.continue/config.yaml；后续可适配 apiBase"],
    },
    detect: detect_continue,
};

pub(crate) static CURSOR: PassiveAgent = PassiveAgent {
    descriptor: AgentDescriptor {
        id: "cursor",
        name: "Cursor",
        config_format: "json",
        supports_write: false,
        notes: &["仅 BYOK 通道可接管；需系统代理兜底"],
    },
    detect: detect_cursor,
};

pub(crate) static VSCODE: PassiveAgent = PassiveAgent {
    descriptor: AgentDescriptor {
        id: "vscode",
        name: "VS Code Copilot Chat",
        config_format: "json",
        supports_write: false,
        notes: &["BYOK 仅覆盖 Chat；补全/检索走 Copilot 服务"],
    },
    detect: detect_vscode,
};

pub(crate) static CLINE: PassiveAgent = PassiveAgent {
    descriptor: AgentDescriptor {
        id: "cline",
        name: "Cline",
        config_format: "extension-storage",
        supports_write: false,
        notes: &["VS Code globalStorage；待适配"],
    },
    detect: detect_cline,
};

pub(crate) static ROO_CODE: PassiveAgent = PassiveAgent {
    descriptor: AgentDescriptor {
        id: "roo_code",
        name: "Roo Code",
        config_format: "extension-storage",
        supports_write: false,
        notes: &["VS Code globalStorage；待适配"],
    },
    detect: detect_roo,
};
