//! 成本计算纯函数（proxy / policy 复用；无 DB）。
//!
//! 公式与舍入口径见 `设计文档（暂未开源）` §5.1：
//!
//! ```text
//! native_cost = (input_tokens × unit_price_input + output_tokens × unit_price_output) / 1_000_000
//! cost_cny    = price_currency == "USD" ? native_cost × fx_rate : native_cost
//! cost        = round_half_up(cost_cny, 6)   # 仅最后一步舍入
//! ```
//!
//! - 单价单位：**每 1,000,000 tokens**；缺失单价按 0 计算；
//! - 金额用 `Decimal`（禁止 f64）；中间计算不提前舍入；
//! - `fx_rate` 来自 `GW_USD_CNY_RATE`，仅 USD 价目使用（并写 `usage_records.fx_rate` 快照）。

use rust_decimal::{Decimal, RoundingStrategy};

use super::model::{PRICE_CURRENCY_CNY, PRICE_CURRENCY_USD};

/// 账本金额小数位（`NUMERIC(18,6)`）。
const COST_SCALE: u32 = 6;
/// 单价单位：每 1,000,000 tokens。
const TOKENS_PER_PRICE_UNIT: i64 = 1_000_000;

/// 价目快照（每 1M tokens 单价 + 币种）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceBasis {
    pub input: Option<Decimal>,
    pub output: Option<Decimal>,
    /// `CNY` / `USD`（大小写不敏感）。
    pub currency: String,
}

/// 成本计算错误（概率极低；已由 providers 校验的输入不会触发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CostError {
    #[error("cost computation overflowed")]
    Overflow,
    #[error("invalid price basis")]
    InvalidPrice,
}

/// 计算一次请求的 CNY 成本（四舍五入到 6 位小数，HALF_UP）。
///
/// - 负 token / 负单价 / 未知币种 → [`CostError::InvalidPrice`]；
/// - USD 价目要求 `fx_rate > 0`；
/// - 乘法/加法超出 `Decimal` 范围 → [`CostError::Overflow`]。
pub fn compute_cost(
    in_tokens: i64,
    out_tokens: i64,
    price: &PriceBasis,
    fx_rate: Decimal,
) -> Result<Decimal, CostError> {
    let price_currency = price.currency.trim().to_ascii_uppercase();
    let is_usd = match price_currency.as_str() {
        PRICE_CURRENCY_CNY => false,
        PRICE_CURRENCY_USD => true,
        _ => return Err(CostError::InvalidPrice),
    };
    if in_tokens < 0 || out_tokens < 0 {
        return Err(CostError::InvalidPrice);
    }
    if is_usd && fx_rate <= Decimal::ZERO {
        return Err(CostError::InvalidPrice);
    }

    let unit_input = price.input.unwrap_or(Decimal::ZERO);
    let unit_output = price.output.unwrap_or(Decimal::ZERO);
    if unit_input.is_sign_negative() || unit_output.is_sign_negative() {
        return Err(CostError::InvalidPrice);
    }

    // 先乘后除：中间不提前舍入（Decimal 28 位有效数字足够）。
    let input_cost = Decimal::from(in_tokens)
        .checked_mul(unit_input)
        .ok_or(CostError::Overflow)?;
    let output_cost = Decimal::from(out_tokens)
        .checked_mul(unit_output)
        .ok_or(CostError::Overflow)?;
    let mut native = input_cost
        .checked_add(output_cost)
        .ok_or(CostError::Overflow)?;
    if is_usd {
        native = native.checked_mul(fx_rate).ok_or(CostError::Overflow)?;
    }
    let cost_cny = native
        .checked_div(Decimal::from(TOKENS_PER_PRICE_UNIT))
        .ok_or(CostError::Overflow)?;
    // 仅最后一步舍入；正数下 MidpointAwayFromZero 即 HALF_UP。
    Ok(cost_cny.round_dp_with_strategy(COST_SCALE, RoundingStrategy::MidpointAwayFromZero))
}

/// 预算预检用估算成本（与 [`compute_cost`] 同一公式/舍入，避免“估算放行、记账超额”口径漂移）。
pub fn estimate_cost(
    price: &PriceBasis,
    in_tokens: i64,
    out_tokens: i64,
    fx_rate: Decimal,
) -> Result<Decimal, CostError> {
    compute_cost(in_tokens, out_tokens, price, fx_rate)
}

/// 字符数粗估 token：`ceil(chars / 3)`（仅供 `usage_source=estimated`；误差可达 ±30%）。
pub fn estimate_tokens(chars: usize) -> i64 {
    let estimate = chars / 3 + usize::from(!chars.is_multiple_of(3));
    i64::try_from(estimate).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::FromPrimitive;
    use std::str::FromStr;

    fn cny(input: Option<Decimal>, output: Option<Decimal>) -> PriceBasis {
        PriceBasis {
            input,
            output,
            currency: PRICE_CURRENCY_CNY.to_string(),
        }
    }

    fn dec(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn computes_cny_cost_per_million_tokens() {
        // 1M 输入 @3 CNY/1M + 500k 输出 @15 CNY/1M = 3 + 7.5 = 10.5
        let price = cny(Some(dec(3)), Some(dec(15)));
        assert_eq!(
            compute_cost(1_000_000, 500_000, &price, Decimal::ONE).unwrap(),
            Decimal::from_f64(10.5).unwrap()
        );
        assert_eq!(
            compute_cost(0, 0, &price, Decimal::ONE).unwrap(),
            Decimal::ZERO
        );
        // 1 token @3/1M = 0.000003
        assert_eq!(
            compute_cost(1, 0, &price, Decimal::ONE).unwrap(),
            Decimal::from_str("0.000003").unwrap()
        );
    }

    #[test]
    fn rounds_half_up_to_six_places() {
        // 1 token × 0.5 CNY/1M = 0.0000005 → HALF_UP → 0.000001
        let price = PriceBasis {
            input: Some(Decimal::from_str("0.5").unwrap()),
            output: None,
            currency: "CNY".to_string(),
        };
        assert_eq!(
            compute_cost(1, 0, &price, Decimal::ONE).unwrap(),
            Decimal::from_str("0.000001").unwrap()
        );
        // 1 token × 0.4 CNY/1M = 0.0000004 → 0.000000
        let price = PriceBasis {
            input: Some(Decimal::from_str("0.4").unwrap()),
            output: None,
            currency: "CNY".to_string(),
        };
        assert_eq!(
            compute_cost(1, 0, &price, Decimal::ONE).unwrap(),
            Decimal::ZERO
        );
    }

    #[test]
    fn converts_usd_price_with_fx_rate() {
        let price = PriceBasis {
            input: Some(Decimal::from_str("1").unwrap()),
            output: Some(Decimal::from_str("2").unwrap()),
            currency: "USD".to_string(),
        };
        let fx = Decimal::from_str("7.2").unwrap();
        // (1M×1 + 0×2) / 1M = 1 USD → 7.2 CNY
        assert_eq!(
            compute_cost(1_000_000, 0, &price, fx).unwrap(),
            Decimal::from_str("7.2").unwrap()
        );
        // 500k 输出 @2 USD/1M = 1 USD → 7.2 CNY
        assert_eq!(
            compute_cost(0, 500_000, &price, fx).unwrap(),
            Decimal::from_str("7.2").unwrap()
        );
        // 币种大小写不敏感
        let upper = PriceBasis {
            currency: "USD".to_string(),
            ..price.clone()
        };
        assert_eq!(
            compute_cost(1_000_000, 0, &upper, fx).unwrap(),
            Decimal::from_str("7.2").unwrap()
        );
    }

    #[test]
    fn missing_price_or_tokens_is_zero_cost() {
        let price = cny(None, None);
        assert_eq!(
            compute_cost(1_000_000, 500_000, &price, Decimal::ONE).unwrap(),
            Decimal::ZERO
        );
        let price = cny(Some(dec(3)), None);
        assert_eq!(
            compute_cost(0, 0, &price, Decimal::ONE).unwrap(),
            Decimal::ZERO
        );
    }

    #[test]
    fn invalid_price_basis_is_rejected() {
        let eur = PriceBasis {
            input: Some(dec(1)),
            output: None,
            currency: "EUR".to_string(),
        };
        assert_eq!(
            compute_cost(1, 0, &eur, Decimal::ONE),
            Err(CostError::InvalidPrice)
        );
        let negative_price = cny(Some(dec(-1)), None);
        assert_eq!(
            compute_cost(1, 0, &negative_price, Decimal::ONE),
            Err(CostError::InvalidPrice)
        );
        assert_eq!(
            compute_cost(-1, 0, &cny(Some(dec(1)), None), Decimal::ONE),
            Err(CostError::InvalidPrice)
        );
        // USD 价目必须带正汇率
        let usd = PriceBasis {
            input: Some(dec(1)),
            output: None,
            currency: "USD".to_string(),
        };
        assert_eq!(
            compute_cost(1, 0, &usd, Decimal::ZERO),
            Err(CostError::InvalidPrice)
        );
    }

    #[test]
    fn overflow_returns_cost_error() {
        let huge = Decimal::from(i64::MAX);
        let price = cny(Some(huge), Some(huge));
        assert_eq!(
            compute_cost(i64::MAX, i64::MAX, &price, Decimal::ONE),
            Err(CostError::Overflow)
        );
    }

    #[test]
    fn estimate_matches_compute_cost() {
        let price = cny(Some(dec(3)), Some(dec(15)));
        let fx = Decimal::from_str("7.2").unwrap();
        assert_eq!(
            estimate_cost(&price, 1234, 567, fx).unwrap(),
            compute_cost(1234, 567, &price, fx).unwrap()
        );
    }

    #[test]
    fn estimate_tokens_rounds_up_by_three_chars() {
        assert_eq!(estimate_tokens(0), 0);
        assert_eq!(estimate_tokens(1), 1);
        assert_eq!(estimate_tokens(2), 1);
        assert_eq!(estimate_tokens(3), 1);
        assert_eq!(estimate_tokens(4), 2);
        assert_eq!(estimate_tokens(300), 100);
        // CJK 按字符（非字节）计。
        assert_eq!(estimate_tokens("你好世界".chars().count()), 2);
        // 极端值按 ceil(chars/3) 计算（64 位平台结果在 i64 范围内）。
        assert_eq!(estimate_tokens(usize::MAX), (usize::MAX / 3) as i64);
    }
}
