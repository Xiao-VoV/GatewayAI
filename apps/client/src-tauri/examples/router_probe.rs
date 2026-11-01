//! 开发/诊断用：不启动 GUI，仅用环境变量启动本地路由并输出 base URL。
//! 用法：
//!   GW_PROBE_SERVER_URL=http://127.0.0.1:8080 \
//!   GW_PROBE_DEVICE_ID=... GW_PROBE_DEVICE_SECRET=... GW_PROBE_API_KEY=... \
//!   cargo run --example router_probe

use std::io::Write;

use gw_client_lib::router::{start, RouterConfig};

#[tokio::main]
async fn main() {
    let config = RouterConfig {
        server_url: std::env::var("GW_PROBE_SERVER_URL").expect("GW_PROBE_SERVER_URL"),
        device_id: std::env::var("GW_PROBE_DEVICE_ID").expect("GW_PROBE_DEVICE_ID"),
        device_secret: std::env::var("GW_PROBE_DEVICE_SECRET").expect("GW_PROBE_DEVICE_SECRET"),
        api_key: std::env::var("GW_PROBE_API_KEY").expect("GW_PROBE_API_KEY"),
    };
    let router = start(config).await.expect("start local router");
    println!("READY {}", router.info.base_url);
    std::io::stdout().flush().ok();
    let _ = tokio::signal::ctrl_c().await;
    drop(router);
}
