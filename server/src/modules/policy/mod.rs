//! policy：用户日预算 + 内部预算预检（S3）；RPM/TPM 延后 v0.1.x。
//!
//! 设计：`设计文档（暂未开源）`（V0.2 已评审）。
//! 预算管理台 API + 供 S4 proxy 使用的内部 `check_budget`；不写 usage/users。
//! S3.1 起：供 proxy 使用的 Key RPM 进程内滑动窗口 `rate_limit::check_key_rpm`。

pub mod model;
pub mod money;
pub mod rate_limit;
pub mod repo;
pub mod routes;
pub mod service;

pub use routes::router;
