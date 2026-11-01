//! 路由目标稳定接缝：把 routing 的解析结果映射为 proxy 内部结构。
//!
//! routing 解析链可以是 legacy alias（`source=Alias`）或 catalog 模型组
//! （`source=Catalog`，带 model_group/deployment/固定账号/catalog_version）；
//! catalog 字段在 routing `RoutePlan`/`RouteCandidate` 中为 Option，proxy 按 Option 读取。

use rust_decimal::Decimal;
use uuid::Uuid;

use crate::modules::routing::model::{
    RouteCandidate, RoutePlan, RouteSource as RoutingRouteSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    Alias,
    Catalog,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // 稳定接缝：v0.1.x catalog 字段与内部元数据供后续消费
pub struct PlannedTarget {
    pub upstream_model_id: Uuid,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub provider_kind: String,
    pub base_url: String,
    pub upstream_model: String,
    pub canonical: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i32>,
    pub max_output: Option<i32>,
    pub price_input: Option<Decimal>,
    pub price_output: Option<Decimal>,
    pub currency: String,
    pub priority: i32,
    pub weight: i32,
    pub source: RouteSource,
    pub model_group_id: Option<Uuid>,
    pub deployment_id: Option<Uuid>,
    pub provider_account_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // requested_model 供日志/排障扩展
pub struct PlannedRoute {
    pub requested_model: String,
    pub model_alias: String,
    /// catalog 来源的对外模型组名（=请求别名）；alias 来源为请求的 alias。
    pub source: RouteSource,
    pub model_group_id: Option<Uuid>,
    pub catalog_version: Option<String>,
    pub targets: Vec<PlannedTarget>,
}

impl From<RoutePlan> for PlannedRoute {
    fn from(plan: RoutePlan) -> Self {
        let source = match plan.source {
            RoutingRouteSource::Alias => RouteSource::Alias,
            RoutingRouteSource::Catalog => RouteSource::Catalog,
        };
        Self {
            requested_model: plan.requested_model,
            model_alias: plan.alias,
            source,
            model_group_id: plan.model_group_id,
            catalog_version: plan.catalog_version,
            targets: plan
                .candidates
                .into_iter()
                .map(|candidate| PlannedTarget::from_candidate(candidate, source))
                .collect(),
        }
    }
}

impl From<RouteCandidate> for PlannedTarget {
    fn from(candidate: RouteCandidate) -> Self {
        Self::from_candidate(candidate, RouteSource::Alias)
    }
}

impl PlannedTarget {
    fn from_candidate(candidate: RouteCandidate, source: RouteSource) -> Self {
        Self {
            upstream_model_id: candidate.upstream_model_id,
            provider_id: candidate.provider_id,
            provider_name: candidate.provider_name,
            provider_kind: candidate.provider_kind,
            base_url: candidate.base_url,
            upstream_model: candidate.upstream_model,
            canonical: candidate.canonical,
            capabilities: candidate.capabilities,
            context_window: candidate.context_window,
            max_output: candidate.max_output,
            price_input: candidate.price_input,
            price_output: candidate.price_output,
            currency: candidate.currency,
            priority: candidate.priority,
            weight: candidate.weight,
            source,
            model_group_id: candidate.model_group_id,
            deployment_id: candidate.deployment_id,
            provider_account_id: candidate.provider_account_id,
        }
    }
}

impl PlannedTarget {
    /// 上游 `/chat/completions`（或原生 `/messages`）的完整 URL；base_url 由 providers 校验且含版本前缀。
    pub fn completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> RouteCandidate {
        RouteCandidate {
            upstream_model_id: Uuid::new_v4(),
            provider_id: Uuid::new_v4(),
            provider_name: "DeepSeek".to_string(),
            provider_kind: "openai_compatible".to_string(),
            base_url: "http://127.0.0.1:8081/v1".to_string(),
            upstream_model: "deepseek-chat".to_string(),
            canonical: "deepseek-chat".to_string(),
            capabilities: vec!["chat".to_string(), "stream".to_string()],
            context_window: Some(65536),
            max_output: Some(8192),
            price_input: None,
            price_output: None,
            currency: "CNY".to_string(),
            priority: 1,
            weight: 100,
            deployment_id: None,
            provider_account_id: None,
            model_group_id: None,
        }
    }

    #[test]
    fn candidate_maps_to_planned_target() {
        let plan = PlannedRoute::from(RoutePlan {
            requested_model: "fast".to_string(),
            source: RoutingRouteSource::Alias,
            model_group_id: None,
            alias_id: Some(Uuid::new_v4()),
            alias: "fast".to_string(),
            catalog_version: None,
            candidates: vec![candidate()],
        });
        assert_eq!(plan.model_alias, "fast");
        assert_eq!(plan.targets.len(), 1);
        let target = &plan.targets[0];
        assert_eq!(target.source, RouteSource::Alias);
        assert!(target.deployment_id.is_none());
        assert_eq!(
            target.completions_url(),
            "http://127.0.0.1:8081/v1/chat/completions"
        );
    }
}
