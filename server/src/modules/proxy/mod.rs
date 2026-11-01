//! proxy：数据面推理入口（OpenAI Chat / Anthropic Messages / OpenAI Responses）与协议适配。
//!
//! 设计：`设计文档（暂未开源）`（技术设计 V0.2 / 接口设计 / 接口契约）。
//! 本模块负责：双因子鉴权、Key RPM、请求校验、Key 白名单、catalog 授权、routing 解析、预算预检、
//! providers 账号/凭证选择、上游 HTTP/SSE、Chat 透传、Messages↔Chat 转换、
//! 2 次尝试/fallback、错误清洗、usage/request_log 事件、响应头。
//!
//! 不负责：`/v1/models*`（catalog owner）、管理台鉴权、路由/预算/计量/日志的持久化。

pub mod adapters;
pub mod auth;
pub mod budget;
pub mod error;
pub mod executor;
#[cfg(test)]
mod integration_tests;
pub mod route;
pub mod routes;
pub mod settings;
pub mod sse;
pub mod stream;
pub mod telemetry;
pub mod upstream;

use crate::platform::state::AppState;
use axum::Router;

/// 数据面推理入口（`/v1/models*` 由 catalog 提供，proxy 不注册）。
pub fn router() -> Router<AppState> {
    routes::router()
}
