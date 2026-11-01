//! 本地路由 sidecar：127.0.0.1 入口，注入虚拟 Key + 设备 token 后转发企业网关。
//!
//! 模块边界：
//! - `mod.rs`：配置/生命周期/manager；
//! - `token.rs`：token 缓存与刷新；
//! - `proxy.rs`：HTTP 转发、header 处理、流式响应。

mod proxy;
mod token;

use std::sync::Arc;
use std::time::Duration;

use axum::{routing::any, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};

const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterConfig {
    pub server_url: String,
    pub device_id: String,
    pub device_secret: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterInfo {
    pub port: u16,
    pub base_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub base_url: Option<String>,
}

pub struct LocalRouter {
    shutdown: Option<oneshot::Sender<()>>,
    pub info: RouterInfo,
}

impl Drop for LocalRouter {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
    }
}

/// 进程级路由生命周期管理；命令层只调用这里，不直接持有 handle。
#[derive(Default)]
pub struct LocalRouterManager {
    current: std::sync::Mutex<Option<LocalRouter>>,
}

impl LocalRouterManager {
    pub async fn start(&self, config: RouterConfig) -> Result<RouterInfo, String> {
        let old = self
            .current
            .lock()
            .map_err(|_| "本地路由状态锁不可用".to_string())?
            .take();
        drop(old);
        let started = start(config).await?;
        let info = started.info.clone();
        let mut slot = self
            .current
            .lock()
            .map_err(|_| "本地路由状态锁不可用".to_string())?;
        *slot = Some(started);
        Ok(info)
    }

    pub fn stop(&self) -> Result<(), String> {
        let old = self
            .current
            .lock()
            .map_err(|_| "本地路由状态锁不可用".to_string())?
            .take();
        drop(old);
        Ok(())
    }

    pub fn status(&self) -> RouterStatus {
        match self.current.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(router) => RouterStatus {
                    running: true,
                    port: Some(router.info.port),
                    base_url: Some(router.info.base_url.clone()),
                },
                None => RouterStatus {
                    running: false,
                    port: None,
                    base_url: None,
                },
            },
            Err(_) => RouterStatus {
                running: false,
                port: None,
                base_url: None,
            },
        }
    }
}

pub(crate) struct RouterState {
    pub(crate) gateway: String,
    pub(crate) device_id: String,
    pub(crate) device_secret: String,
    pub(crate) api_key: String,
    pub(crate) client: reqwest::Client,
    pub(crate) token: Mutex<Option<token::TokenCache>>,
}

impl RouterState {
    fn new(config: RouterConfig) -> Result<Arc<Self>, String> {
        let gateway = config.server_url.trim_end_matches('/').to_string();
        if !(gateway.starts_with("http://") || gateway.starts_with("https://")) {
            return Err("网关地址必须是 http/https".to_string());
        }
        if config.api_key.is_empty()
            || config.device_secret.is_empty()
            || config.device_id.is_empty()
        {
            return Err("本地路由缺少设备凭据".to_string());
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(Self {
            gateway,
            device_id: config.device_id,
            device_secret: config.device_secret,
            api_key: config.api_key,
            client,
            token: Mutex::new(None),
        }))
    }
}

pub async fn start(config: RouterConfig) -> Result<LocalRouter, String> {
    let state = RouterState::new(config)?;
    let app = Router::new()
        .route("/v1", any(proxy::handler))
        .route("/v1/{*path}", any(proxy::handler))
        .route("/v1beta", any(proxy::handler))
        .route("/v1beta/{*path}", any(proxy::handler))
        .with_state(state);
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("本地路由端口绑定失败：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = server.await {
            eprintln!("local router stopped: {error}");
        }
    });
    Ok(LocalRouter {
        shutdown: Some(shutdown_tx),
        info: RouterInfo {
            port,
            base_url: format!("http://127.0.0.1:{port}"),
        },
    })
}
