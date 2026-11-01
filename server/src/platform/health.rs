//! 健康检查、就绪检查与最小指标。

use crate::platform::state::AppState;
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::timeout;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
}

async fn healthz() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "gw-server",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let (postgres_ok, postgres_detail) = match &state.db {
        None => (false, "not_configured"),
        Some(pool) => {
            match timeout(
                Duration::from_secs(2),
                sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool),
            )
            .await
            {
                Ok(Ok(_)) => (true, "ok"),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "readiness postgres check failed");
                    (false, "connect_failed")
                }
                Err(_) => (false, "timeout"),
            }
        }
    };

    let status = if postgres_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(json!({
            "status": if postgres_ok { "ready" } else { "not_ready" },
            "checks": {
                "postgres": postgres_detail,
            },
        })),
    )
}

async fn metrics(State(state): State<AppState>) -> String {
    let uptime = state.started_at.elapsed().as_secs();
    let db_configured = u8::from(state.db.is_some());
    let bind = state.config.bind;
    format!(
        "# HELP gw_up 服务是否存活\n# TYPE gw_up gauge\ngw_up 1\n\
         # HELP gw_uptime_seconds 进程运行秒数\n# TYPE gw_uptime_seconds gauge\ngw_uptime_seconds {uptime}\n\
         # HELP gw_db_configured 是否配置数据库\n# TYPE gw_db_configured gauge\ngw_db_configured {db_configured}\n\
         # HELP gw_server_info 服务监听地址\n# TYPE gw_server_info gauge\ngw_server_info{{bind=\"{bind}\"}} 1\n"
    )
}
