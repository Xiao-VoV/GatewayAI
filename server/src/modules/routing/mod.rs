#![allow(dead_code)] // resolve_model/visible_models/RoutePlan 由 S4 proxy 接入；接入后移除。

//! routing：多目标模型别名、路由计划、可见模型投影。
//!
//! 设计：`设计文档（暂未开源）`（V0.2 已评审）。
//! 本模块提供管理台别名 API 与内部 resolve/visible_models；proxy 执行 RoutePlan。

pub mod model;
pub mod repo;
pub mod routes;
pub mod service;

pub use routes::router;
