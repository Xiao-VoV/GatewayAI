use tauri::Manager;

use crate::agents::{
    self, AgentApplyResult, AgentConfigStatus, AgentContext, AgentPlan, AgentScan, ConfigRequest,
};

fn context(app: &tauri::AppHandle) -> Result<AgentContext, String> {
    let home =
        crate::agents::context::home_dir().ok_or_else(|| "无法读取用户主目录".to_string())?;
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("读取应用数据目录失败：{error}"))?;
    Ok(AgentContext::new(home, app_data))
}

fn adapter(agent_id: &str) -> Result<&'static dyn agents::AgentAdapter, String> {
    agents::find(agent_id).ok_or_else(|| format!("未知 Agent：{agent_id}"))
}

#[tauri::command]
pub fn scan_agents(app: tauri::AppHandle) -> Result<AgentScan, String> {
    Ok(agents::scan(&context(&app)?))
}

#[tauri::command]
pub fn preview_agent_config(
    app: tauri::AppHandle,
    agent_id: String,
    local_base_url: String,
    model: Option<String>,
) -> Result<AgentPlan, String> {
    let ctx = context(&app)?;
    adapter(&agent_id)?.plan(
        &ctx,
        &ConfigRequest {
            local_base_url,
            model,
        },
    )
}

#[tauri::command]
pub fn apply_agent_config(
    app: tauri::AppHandle,
    agent_id: String,
    local_base_url: String,
    model: Option<String>,
) -> Result<AgentApplyResult, String> {
    let ctx = context(&app)?;
    adapter(&agent_id)?.apply(
        &ctx,
        &ConfigRequest {
            local_base_url,
            model,
        },
    )
}

#[tauri::command]
pub fn rollback_agent_config(app: tauri::AppHandle, agent_id: String) -> Result<(), String> {
    let ctx = context(&app)?;
    adapter(&agent_id)?.rollback(&ctx)
}

#[tauri::command]
pub fn agent_config_status(
    app: tauri::AppHandle,
    agent_id: String,
) -> Result<AgentConfigStatus, String> {
    let ctx = context(&app)?;
    Ok(adapter(&agent_id)?.config_status(&ctx))
}
