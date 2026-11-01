//! catalog：模型目录/模型组/部署/发布（v0.1.x）与 S4 客户端目录过渡。
//!
//! 客户端：`GET /v1/models` / `GET /v1/models/{id}` 走 catalog 发布投影（`cat:<n>`）；
//! 迭代 3 起：routing 通过 `service::routing_group_views` 做 catalog-first 解析，
//! proxy 通过 `service::authorize_model` 做发布授权（catalog 不反向依赖 routing）。
//! v0.1.x 控制面：模型组 + 部署 + 发布 16 个管理台端点（聚合建议/工具默认/员工预览后续）。
//! `/v1/models*` HTTP owner = catalog（proxy 不注册）。

mod cache;
pub mod model;
pub mod repo;
pub mod routes;
pub mod service;

use crate::platform::state::AppState;
use axum::Router;

/// 客户端模型目录（S4 过渡；v0.1.x 切换发布投影）。
pub fn client_router() -> Router<AppState> {
    routes::client_router()
}

/// 管理台模型目录/模型组/发布（v0.1.x，当前 501）。
pub fn admin_router() -> Router<AppState> {
    routes::admin_router()
}
