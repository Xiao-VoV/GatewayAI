//! providers：供应商、渠道账号、上游模型、凭证加密与连通性测试。
//!
//! 设计：`设计文档（暂未开源）`（技术设计 / 接口设计 / 数据库设计 / 接口契约）。
//! S2 范围：US-004/005/006/034；仅管理台，客户端面不可达。

pub mod connectivity;
pub mod crypto;
pub mod model;
pub mod repo;
pub mod routes;
pub mod service;
pub mod validate;

pub use routes::router;
