//! Orveil Desktop Tauri composition root.
//!
//! 分层：commands（薄命令）→ agents/config_tx/platform/router（业务）；
//! 桌面托盘/窗口在 desktop；这里只做模块装配与插件注册。

pub mod agents;
mod commands;
mod config_tx;
mod desktop;
pub mod platform;
pub mod router;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            desktop::show_main_window(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(router::LocalRouterManager::default())
        .setup(desktop::setup)
        .on_window_event(desktop::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::credentials::save_credentials,
            commands::credentials::load_credentials,
            commands::credentials::clear_credentials,
            commands::agents::scan_agents,
            commands::agents::preview_agent_config,
            commands::agents::apply_agent_config,
            commands::agents::rollback_agent_config,
            commands::agents::agent_config_status,
            commands::router::start_local_router,
            commands::router::stop_local_router,
            commands::router::local_router_status,
            commands::system::native_http_request
        ])
        .run(tauri::generate_context!())
        .expect("error while running Orveil Desktop");
}
