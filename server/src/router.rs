//! HTTP 路由装配。仅做 merge，不在 main.rs 里堆路由。

use crate::{modules, platform::state::AppState};
use axum::{Router, middleware};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub fn app(state: AppState) -> Router {
    let request_id_header = crate::platform::http::request_id_header();

    Router::new()
        // platform
        .merge(platform_health())
        // admin + client
        .merge(modules::identity::router())
        .merge(modules::providers::router())
        .merge(modules::access::client_router())
        .merge(modules::access::admin_router())
        .merge(modules::policy::router())
        .merge(modules::routing::router())
        .merge(modules::metering::router())
        .merge(modules::metering::client_router())
        .merge(modules::observability::router())
        .merge(modules::catalog::client_router())
        .merge(modules::catalog::admin_router())
        .merge(modules::settings::admin_router())
        .merge(modules::proxy::router())
        // 管理写审计：置于 request-id 层之内（能读到 x-request-id），响应不改写。
        .layer(middleware::from_fn_with_state(
            state.clone(),
            modules::observability::audit::admin_audit_middleware,
        ))
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn platform_health() -> Router<AppState> {
    crate::platform::health::router()
}
