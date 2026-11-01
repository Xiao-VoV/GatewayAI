//! access：员工用户、配对码、设备、设备 token、一设备一虚拟 Key。
//!
//! 设计：`设计文档（暂未开源）`（V0.2 已评审）。
//! 管理台与客户端是两组路由；S4 proxy 通过本模块 service 内部接口鉴权。

pub mod model;
pub mod rate_limit;
pub mod repo;
pub mod routes;
pub mod security;
pub mod service;

pub use routes::{admin_router, client_router};
