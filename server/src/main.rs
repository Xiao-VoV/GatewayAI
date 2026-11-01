//! gw-server：Axum 服务端骨架。
//!
//! 模块划分、边界与实现顺序以 `设计文档（暂未开源）` 为准：
//!   platform + modules/{identity,providers,access,policy,routing,proxy,
//!   metering,observability,catalog}
//!
//! 已实现：identity/providers/access/policy/routing/proxy（S4 数据面）；
//! metering/observability 的 S4 写入；catalog 与 S5 查询端点仍为 501，按切片推进。
//!
//! 数据库：PostgreSQL（v0.1 直接上，不做 SQLite 迁移）；缓存按需 Redis。

mod modules;
mod platform;
mod router;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod http_contract_tests;

use anyhow::Context;
use platform::{config::Config, crypto::Crypto, db, state::AppState};
use std::{net::SocketAddr, sync::Arc};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    // 允许从仓库根目录 `cargo run --manifest-path server/Cargo.toml` 启动。
    if std::env::var("GW_DATABASE_URL").is_err() && std::env::var("DATABASE_URL").is_err() {
        dotenvy::from_filename("server/.env").ok();
    }

    let config = Config::from_env().context("invalid server configuration")?;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.log_filter.clone().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!(bind = %config.bind, "starting gw-server");

    // 部署告警（不改变启动行为，US-032 的启动必填校验留给 platform v0.1.1）。
    if !config.cookie_secure && !config.bind.ip().is_loopback() {
        tracing::warn!("GW_COOKIE_SECURE=false 且监听非回环地址：生产环境必须开启 Secure Cookie");
    }
    if let Some(token) = &config.setup_token
        && token.chars().count() < 16
    {
        tracing::warn!("GW_SETUP_TOKEN 少于 16 个字符，建议使用更长的高熵随机值");
    }

    let database = db::connect(config.database_url.as_deref()).await?;
    if let (Some(pool), true) = (&database, config.run_migrations) {
        match db::migrate(pool).await {
            Ok(()) => tracing::info!("database migrations applied"),
            Err(error) => {
                tracing::error!(%error, "database migration failed; /readyz will stay not-ready")
            }
        }
    }

    let master_key = Crypto::load().context("invalid master key configuration")?;

    let addr = config.bind;
    let state = AppState::new(Arc::new(config), database, Arc::new(master_key));
    // 平台设置：DB 权威，启动加载 runtime（DB 不可用时 env/默认兜底并 warn）。
    modules::settings::service::initialize(&state).await;
    modules::metering::service::spawn_maintenance(state.clone());
    modules::observability::service::spawn_cleanup(state.clone());
    let app = router::app(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    tracing::info!(%addr, "gw-server listening");

    let serve_result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;

    // 优雅退出：等待 metering/observability 的旁路写入排空（各 ≤5s，总计有界）。
    let flush = std::time::Duration::from_secs(5);
    let (usage_ok, log_ok) = tokio::join!(
        modules::metering::service::flush(flush),
        modules::observability::service::flush(flush),
    );
    if !usage_ok || !log_ok {
        tracing::warn!(usage_ok, log_ok, "background telemetry flush timed out");
    }

    serve_result?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler")
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
