//! policy 金额/预算判定工具：十进制解析、6 位格式化、日界 retry-after。

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

/// S3 仅支持 CNY。
pub const CURRENCY_CNY: &str = "CNY";
/// 金额上限：999999999999.999999（NUMERIC(18,6)）。
fn max_daily_limit() -> Decimal {
    Decimal::new(999_999_999_999_999_999, 6)
}
/// 日界固定 Asia/Shanghai（UTC+8）。
const DAY_OFFSET_SECONDS: i32 = 8 * 3600;

/// 解析 API 传入的日限额字符串：非负、最多 12 位整数 + 6 位小数。
pub fn parse_daily_limit(raw: &str) -> Result<Decimal, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("金额不能为空".to_string());
    }
    let (integer, fraction) = match value.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (value, None),
    };
    if !is_valid_integer_part(integer) {
        return Err("金额格式不正确（整数部分 1–12 位，不能有前导零）".to_string());
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return Err("金额格式不正确（小数部分必须为数字）".to_string());
        }
        if fraction.len() > 6 {
            return Err("金额最多 6 位小数".to_string());
        }
    }
    let parsed = Decimal::from_str_exact(value).map_err(|_| "金额格式不正确".to_string())?;
    if parsed.is_sign_negative() {
        return Err("金额不能为负数".to_string());
    }
    if parsed > max_daily_limit() {
        return Err("金额超过上限".to_string());
    }
    Ok(parsed)
}

fn is_valid_integer_part(integer: &str) -> bool {
    if integer.is_empty() || integer.len() > 12 {
        return false;
    }
    if !integer.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    integer == "0" || !integer.starts_with('0')
}

/// 输出统一 6 位小数（用于 API/DTO）。
pub fn format_money(value: Decimal) -> String {
    let scaled = (value * Decimal::from(1_000_000)).round();
    let micros = scaled.to_i64().unwrap_or_default();
    let negative = micros < 0;
    let abs = micros.unsigned_abs();
    format!(
        "{}{}.{:06}",
        if negative { "-" } else { "" },
        abs / 1_000_000,
        abs % 1_000_000
    )
}

/// 预算判定（纯函数，便于单测/多调用方一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetDecision {
    Allowed,
    Exceeded {
        limit: Decimal,
        spent_today: Decimal,
        estimated_cost: Decimal,
        currency: String,
        retry_after_secs: u64,
    },
}

pub fn decide_budget(
    limit: Decimal,
    spent_today: Decimal,
    estimated_cost: Decimal,
    currency: &str,
    now: DateTime<Utc>,
) -> BudgetDecision {
    if limit <= Decimal::ZERO || spent_today + estimated_cost <= limit {
        return BudgetDecision::Allowed;
    }
    BudgetDecision::Exceeded {
        limit,
        spent_today,
        estimated_cost,
        currency: currency.to_string(),
        retry_after_secs: retry_after_secs(now),
    }
}

/// 距下一个 Asia/Shanghai 00:00 的秒数，最小 60。
pub fn retry_after_secs(now: DateTime<Utc>) -> u64 {
    let offset = FixedOffset::east_opt(DAY_OFFSET_SECONDS).expect("valid +08:00 offset");
    let local = now.with_timezone(&offset);
    let Some(tomorrow) = local.date_naive().succ_opt() else {
        return 60;
    };
    let midnight = tomorrow.and_hms_opt(0, 0, 0).expect("valid midnight");
    let next = match offset.from_local_datetime(&midnight).single() {
        Some(value) => value.with_timezone(&Utc),
        None => return 60,
    };
    next.signed_duration_since(now).num_seconds().max(60) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn parses_and_formats_money() {
        assert_eq!(parse_daily_limit("5").unwrap(), Decimal::new(5, 0));
        assert_eq!(parse_daily_limit("5.00").unwrap(), Decimal::new(500, 2));
        assert_eq!(parse_daily_limit("0").unwrap(), Decimal::ZERO);
        assert_eq!(format_money(Decimal::new(5, 0)), "5.000000");
        assert_eq!(format_money(Decimal::new(500, 2)), "5.000000");
        assert_eq!(format_money(Decimal::new(1, 6)), "0.000001");
        assert_eq!(format_money(Decimal::ZERO), "0.000000");
        assert!(parse_daily_limit("01").is_err());
        assert!(parse_daily_limit("-1").is_err());
        assert!(parse_daily_limit("1.1234567").is_err());
        assert!(parse_daily_limit("1e3").is_err());
        assert!(parse_daily_limit("1.2.3").is_err());
        assert!(parse_daily_limit("").is_err());
        assert!(parse_daily_limit("9999999999999").is_err());
    }

    #[test]
    fn zero_or_missing_limit_allows() {
        let now = dt(2026, 10, 30, 6);
        assert_eq!(
            decide_budget(Decimal::ZERO, Decimal::new(5, 0), Decimal::ZERO, "CNY", now),
            BudgetDecision::Allowed
        );
    }

    #[test]
    fn equal_budget_allows_over_budget_blocks() {
        let now = dt(2026, 10, 30, 6);
        assert_eq!(
            decide_budget(
                Decimal::new(5, 0),
                Decimal::new(499, 2),
                Decimal::new(1, 2),
                "CNY",
                now
            ),
            BudgetDecision::Allowed
        );
        assert!(matches!(
            decide_budget(
                Decimal::new(5, 0),
                Decimal::new(499, 2),
                Decimal::new(2, 2),
                "CNY",
                now
            ),
            BudgetDecision::Exceeded { .. }
        ));
    }

    #[test]
    fn retry_after_until_shanghai_midnight() {
        // 2026-10-30 06:00 UTC = 14:00 +08；距次日 00:00 = 10h = 36000s。
        assert_eq!(retry_after_secs(dt(2026, 10, 30, 6)), 36_000);
        assert_eq!(retry_after_secs(dt(2026, 10, 30, 15)).min(60), 60);
    }
}
