//! metering：请求用量、成本与日聚合。
//!
//! 设计：`设计文档（暂未开源）`（V0.1）。
//! S4 提供 proxy 所需的写入/读数内部接口（`record_usage(_detached)`/`flush`/`today_spend`，
//! 以及 `cost` 纯函数）；S5 管理台总览/用量查询；S5.1 日结 rollup、保留清理与维护任务
//! （`finalize_*`/`run_maintenance_once`/`spawn_maintenance`）。
//! 本模块只写 `usage_records`/`daily_usage_rollups`/`usage_rollup_days`，不存 prompt/response。

pub mod cost;
pub mod model;
pub mod repo;
pub mod routes;
pub mod service;

// 以下 re-export 是 proxy S4 接入后的简写入口（设计文档 `metering::today_spend` 等）；
// 接入前本 crate 内无调用方，允许未使用。
#[allow(unused_imports)]
pub use cost::{CostError, PriceBasis, compute_cost, estimate_cost, estimate_tokens};
#[allow(unused_imports)]
pub use model::{RouteSource, UsageEvent, UsageSource, UsageStatus};
pub use routes::{client_router, router};
#[allow(unused_imports)]
pub use service::{
    finalize_day, finalize_due_days, flush, record_usage, record_usage_detached,
    run_maintenance_once, spawn_maintenance, today_spend,
};
