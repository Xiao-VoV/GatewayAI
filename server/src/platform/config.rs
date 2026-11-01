//! 服务配置。只从环境变量/`.env` 读取，不在代码里写死连接串。
//!
//! identity 相关变量见 `设计文档（暂未开源）` §1.4：
//! - `GW_DATABASE_URL` / `DATABASE_URL`：空表示 DB 关闭（仅 smoke，identity 端点返回 503）
//! - `GW_COOKIE_SECURE`：Session Cookie 是否加 Secure（生产必须 true）
//! - `GW_SETUP_TOKEN`：非空时 `/setup` 必须带 `X-Setup-Token`
//!
//! 注意：S1 会话固定 12h（D18），**没有** `GW_SESSION_TTL_SECS`。

use anyhow::{Context, Result};
use rust_decimal::Decimal;
use std::{env, net::SocketAddr, str::FromStr};

#[derive(Debug)]
pub struct Config {
    /// 监听地址，默认 127.0.0.1:8080；生产用 GW_BIND=0.0.0.0:8080。
    pub bind: SocketAddr,
    /// PostgreSQL 连接串；未配置时服务可启动，但 /readyz 返回 503。
    pub database_url: Option<String>,
    /// 默认日志过滤器；可被 RUST_LOG 覆盖。
    pub log_filter: String,
    /// 启动时自动跑 sqlx 迁移；生产建议显式控制。
    pub run_migrations: bool,
    /// identity Session Cookie 是否加 Secure；生产必须为 true。
    pub cookie_secure: bool,
    /// 非空时 `POST /api/admin/auth/setup` 必须带 `X-Setup-Token`。
    pub setup_token: Option<String>,
    /// USD 价目折算 CNY 的固定汇率（S4 近似；默认 7.20，>0）。
    pub usd_cny_rate: Decimal,
    /// 是否信任 `X-Forwarded-For`（默认 false；仅反代部署开启）。
    pub trust_proxy_headers: bool,
    /// 数据面请求体上限（默认 8 MiB）。
    pub proxy_max_body_bytes: usize,
    /// 是否对客户端暴露 `x-gw-provider`（默认 false）。
    pub proxy_expose_provider_header: bool,
    /// usage_records 明细保留天数（默认 180，范围 7–3650）。
    pub usage_retention_days: i64,
    /// daily rollup/元数据保留天数（默认 1825，≥ 明细保留期）。
    pub rollup_retention_days: i64,
    /// rollup 维护 tick 间隔秒（默认 300，最小 60）。
    pub rollup_interval_secs: u64,
    /// request_logs 保留天数（默认 30，1–365）。
    pub request_log_retention_days: i64,
    /// audit_logs 保留天数（默认 180，180–3650）。
    pub audit_retention_days: i64,
    /// observability 清理 tick 秒（默认 3600，最小 60）。
    pub log_cleanup_interval_secs: u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind = env::var("GW_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
        let bind = bind
            .parse::<SocketAddr>()
            .with_context(|| format!("GW_BIND 不是合法地址: {bind}"))?;

        let database_url = env::var("GW_DATABASE_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let log_filter =
            env::var("GW_LOG").unwrap_or_else(|_| "gw_server=debug,tower_http=info".to_string());

        let run_migrations = env::var("GW_MIGRATE")
            .map(|value| !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "no"))
            .unwrap_or(true);

        let cookie_secure = match env::var("GW_COOKIE_SECURE") {
            Ok(value) => parse_bool("GW_COOKIE_SECURE", &value)?,
            Err(_) => false,
        };

        let setup_token = env::var("GW_SETUP_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let usd_cny_rate = match env::var("GW_USD_CNY_RATE") {
            Ok(value) => {
                let rate = Decimal::from_str(value.trim())
                    .with_context(|| format!("GW_USD_CNY_RATE 不是合法小数: {value}"))?;
                if rate <= Decimal::ZERO {
                    anyhow::bail!("GW_USD_CNY_RATE 必须大于 0");
                }
                rate
            }
            Err(_) => Decimal::new(720, 2),
        };
        let trust_proxy_headers = match env::var("GW_TRUST_PROXY_HEADERS") {
            Ok(value) => parse_bool("GW_TRUST_PROXY_HEADERS", &value)?,
            Err(_) => false,
        };
        let proxy_max_body_bytes = match env::var("GW_PROXY_MAX_BODY_BYTES") {
            Ok(value) => value
                .trim()
                .parse::<usize>()
                .with_context(|| format!("GW_PROXY_MAX_BODY_BYTES 不是合法整数: {value}"))?,
            Err(_) => 8 * 1024 * 1024,
        };
        if proxy_max_body_bytes < 1024 {
            anyhow::bail!("GW_PROXY_MAX_BODY_BYTES 不能小于 1024");
        }
        let proxy_expose_provider_header = match env::var("GW_PROXY_EXPOSE_PROVIDER_HEADER") {
            Ok(value) => parse_bool("GW_PROXY_EXPOSE_PROVIDER_HEADER", &value)?,
            Err(_) => false,
        };
        let usage_retention_days = parse_i64("GW_USAGE_RETENTION_DAYS", 180, 7, 3650)?;
        let rollup_retention_days = parse_i64("GW_ROLLUP_RETENTION_DAYS", 1825, 7, 36500)?;
        if rollup_retention_days < usage_retention_days {
            anyhow::bail!("GW_ROLLUP_RETENTION_DAYS 不能小于 GW_USAGE_RETENTION_DAYS");
        }
        let rollup_interval_secs = parse_u64("GW_ROLLUP_INTERVAL_SECS", 300, 60, 86_400)?;
        let request_log_retention_days = parse_i64("GW_REQUEST_LOG_RETENTION_DAYS", 30, 1, 365)?;
        let audit_retention_days = parse_i64("GW_AUDIT_RETENTION_DAYS", 180, 180, 3650)?;
        let log_cleanup_interval_secs =
            parse_u64("GW_LOG_CLEANUP_INTERVAL_SECS", 3600, 60, 86_400)?;

        Ok(Self {
            bind,
            database_url,
            log_filter,
            run_migrations,
            cookie_secure,
            setup_token,
            usd_cny_rate,
            trust_proxy_headers,
            proxy_max_body_bytes,
            proxy_expose_provider_header,
            usage_retention_days,
            rollup_retention_days,
            rollup_interval_secs,
            request_log_retention_days,
            audit_retention_days,
            log_cleanup_interval_secs,
        })
    }
}

fn parse_i64(name: &str, default: i64, min: i64, max: i64) -> Result<i64> {
    let value = match env::var(name) {
        Ok(raw) => raw
            .trim()
            .parse::<i64>()
            .with_context(|| format!("{name} 不是合法整数: {raw}"))?,
        Err(_) => default,
    };
    if !(min..=max).contains(&value) {
        anyhow::bail!("{name} 必须在 {min}–{max} 之间");
    }
    Ok(value)
}

fn parse_u64(name: &str, default: u64, min: u64, max: u64) -> Result<u64> {
    let value = match env::var(name) {
        Ok(raw) => raw
            .trim()
            .parse::<u64>()
            .with_context(|| format!("{name} 不是合法整数: {raw}"))?,
        Err(_) => default,
    };
    if !(min..=max).contains(&value) {
        anyhow::bail!("{name} 必须在 {min}–{max} 之间");
    }
    Ok(value)
}

fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{name} 必须是 true/false（当前值: {value}）"),
    }
}
