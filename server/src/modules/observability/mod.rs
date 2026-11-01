//! observability：请求元数据日志与（v0.1.x）管理审计。
//!
//! 设计：`设计文档（暂未开源）`（V0.1）。
//! S4 提供数据面写入内部接口（`record_request_log(_detached)`/`flush` + sanitizer）；
//! S5 管理台请求日志列表/详情；S5.1 审计写入/查询、requests/audit CSV 导出、
//! request_logs/audit_logs 保留清理（`run_cleanup_once`/`spawn_cleanup`）。
//! `request_logs` 只存元数据，不存正文/Authorization/API Key/token/Cookie。

pub mod audit;
pub mod csv;
pub mod model;
pub mod repo;
pub mod routes;
pub mod sanitize;
pub mod service;

// 以下 re-export 是 proxy/catalog S4 接入后的简写入口；接入前本 crate 内无调用方，允许未使用。
#[allow(unused_imports)]
pub use model::{AuditEvent, ProtocolKind, RequestLogEvent};
pub use routes::router;
#[allow(unused_imports)]
pub use service::{
    flush, record_audit, record_audit_detached, record_request_log, record_request_log_detached,
    run_cleanup_once, spawn_cleanup,
};
