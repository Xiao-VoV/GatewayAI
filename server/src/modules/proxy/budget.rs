//! 预算预检：候选价格估算 `estimated_cost` → metering `today_spend` → policy `check_budget`。
//!
//! 设计：proxy 技术设计 §4.4/§4.5。失败关闭：metering/policy 不可用 → 503，禁止按 0 放行；
//! 预算超限 → 429 `budget_exceeded` + `Retry-After` + `budget` 字段（不调用上游）。

use rust_decimal::Decimal;
use uuid::Uuid;

use crate::modules::metering::cost::{PriceBasis, estimate_cost, estimate_tokens};
use crate::modules::policy::money::BudgetDecision;
use crate::modules::policy::service::check_budget;
use crate::platform::error::{AppError, BudgetProblem};
use crate::platform::state::AppState;

use super::route::PlannedTarget;
use super::settings::{DEFAULT_OUTPUT_TOKENS_ESTIMATE, MAX_OUTPUT_TOKENS_ESTIMATE};

#[derive(Debug, Clone)]
pub struct BudgetEstimate {
    pub estimated_cost: Decimal,
}

/// 用候选价格与请求规模估算本次请求成本（CNY，6 位小数在 metering 纯函数内完成）。
///
/// - 输入 token：请求 JSON 字符串字符数粗估（只用于预算，不是计费值）。
/// - 输出 token：请求显式 max_tokens/max_completion_tokens，受候选 max_output 与硬上限约束。
/// - 价格缺失时 metering 纯函数按 0 处理（只事后阻断）。
pub fn estimate(
    target: &PlannedTarget,
    prompt_chars: usize,
    requested_max_tokens: Option<i64>,
    usd_cny_rate: Decimal,
) -> BudgetEstimate {
    let input_tokens = estimate_tokens(prompt_chars).max(1);
    let mut output_tokens = requested_max_tokens
        .unwrap_or(DEFAULT_OUTPUT_TOKENS_ESTIMATE)
        .max(1);
    if let Some(max_output) = target.max_output {
        output_tokens = output_tokens.min(i64::from(max_output.max(1)));
    }
    output_tokens = output_tokens.min(MAX_OUTPUT_TOKENS_ESTIMATE);
    let price = PriceBasis {
        input: target.price_input,
        output: target.price_output,
        currency: target.currency.clone(),
    };
    let estimated_cost =
        estimate_cost(&price, input_tokens, output_tokens, usd_cny_rate).unwrap_or(Decimal::ZERO);
    BudgetEstimate { estimated_cost }
}

/// 预算判定：`today_spend` 失败或 policy 失败均返回 503（失败关闭）。
pub async fn check(
    state: &AppState,
    user_id: Uuid,
    estimate: &BudgetEstimate,
) -> Result<(), AppError> {
    let spent_today = crate::modules::metering::service::today_spend(state, user_id).await?;
    match check_budget(state, user_id, spent_today, estimate.estimated_cost).await? {
        BudgetDecision::Allowed => Ok(()),
        BudgetDecision::Exceeded {
            limit,
            spent_today,
            estimated_cost,
            currency,
            retry_after_secs,
        } => Err(AppError::throttled(
            "budget_exceeded",
            format!(
                "今日预算已用尽（限额 {limit:.6} {currency}，已用 {spent_today:.6}，预估 {estimated_cost:.6}）"
            ),
            retry_after_secs,
            Some(BudgetProblem {
                daily_limit: limit,
                spent: spent_today,
                estimated: estimated_cost,
                currency,
            }),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::proxy::route::RouteSource;
    use uuid::Uuid;

    fn make_target(price_input: Option<Decimal>, price_output: Option<Decimal>) -> PlannedTarget {
        PlannedTarget {
            upstream_model_id: Uuid::new_v4(),
            provider_id: Uuid::new_v4(),
            provider_name: "Test".to_string(),
            provider_kind: "openai_compatible".to_string(),
            base_url: "http://127.0.0.1:1/v1".to_string(),
            upstream_model: "test-model".to_string(),
            canonical: "test-model".to_string(),
            capabilities: Vec::new(),
            context_window: None,
            max_output: Some(4096),
            price_input,
            price_output,
            currency: "CNY".to_string(),
            priority: 1,
            weight: 100,
            source: RouteSource::Alias,
            model_group_id: None,
            deployment_id: None,
            provider_account_id: None,
        }
    }

    #[test]
    fn estimates_cost_from_1m_token_prices() {
        let target = make_target(Some(Decimal::new(2, 0)), Some(Decimal::new(8, 0)));
        // 3000 字符 → 1000 input tokens；max_tokens 500；2/1M + 8/1M。
        let budget = estimate(&target, 3000, Some(500), Decimal::new(720, 2));
        assert_eq!(budget.estimated_cost, Decimal::new(6, 3)); // 0.006
    }

    #[test]
    fn caps_output_tokens_and_handles_missing_prices() {
        let target = make_target(None, None);
        let budget = estimate(&target, 3, Some(10_000_000), Decimal::new(720, 2));
        assert_eq!(budget.estimated_cost, Decimal::ZERO);
    }
}
