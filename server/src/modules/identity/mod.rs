//! identity：管理员账号、登录/退出、Cookie Session + CSRF。
//!
//! 设计：`设计文档（暂未开源）`（技术设计 / 接口设计 / 数据库设计 / 接口契约）。
//! S1 范围：setup、login、me、logout；单管理员（D20），会话固定 12h（D18）。

pub mod middleware;
pub mod model;
pub mod rate_limit;
pub mod repo;
pub mod routes;
pub mod security;
pub mod service;

pub use routes::router;
