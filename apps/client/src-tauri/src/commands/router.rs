use crate::router::{LocalRouterManager, RouterConfig, RouterInfo, RouterStatus};

#[tauri::command]
pub async fn start_local_router(
    manager: tauri::State<'_, LocalRouterManager>,
    config: RouterConfig,
) -> Result<RouterInfo, String> {
    manager.start(config).await
}

#[tauri::command]
pub fn stop_local_router(manager: tauri::State<'_, LocalRouterManager>) -> Result<(), String> {
    manager.stop()
}

#[tauri::command]
pub fn local_router_status(manager: tauri::State<'_, LocalRouterManager>) -> RouterStatus {
    manager.status()
}
