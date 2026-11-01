//! settings：平台设置持久化（US-024 / S5.3）。
//!
//! 设计见 `设计文档（暂未开源）`：
//! - `app_settings` 单例（id=1）为权威，启动 `initialize` 加载到 runtime；
//! - `GET/PATCH /api/admin/settings`（AdminSession；PATCH 需 CSRF）；
//! - runtime 供 observability 清理（保留天数）与 proxy（上游超时）热路径读取。
//!
//! 默认供应商仅持久化/校验/展示；兜底路由语义留 v0.1.x。

pub mod model;
pub mod repo;
pub mod routes;
pub mod runtime;
pub mod service;

pub use routes::admin_router;
