//! settings 运行期快照：DB `app_settings` 为权威，进程内 `RwLock` 供 proxy/observability 热路径读取。
//!
//! - 启动：`service::initialize` 从 DB 加载后 `set`；DB 不可用/缺行 → env/默认兜底并 warn；
//! - 写入：PATCH 事务提交后 `set`（下一次 tick/请求生效）；
//! - 默认值 30/180/60（与迁移 0012 一致）；env 仅在 DB 不可用时兜底（评审 Medium）。

use std::sync::{OnceLock, RwLock};

use chrono::Utc;

use crate::platform::config::Config;

use super::model::{
    DEFAULT_AUDIT_RETENTION_DAYS, DEFAULT_REQUEST_LOG_RETENTION_DAYS,
    DEFAULT_UPSTREAM_TIMEOUT_SECS, MAX_AUDIT_RETENTION_DAYS, MAX_REQUEST_LOG_RETENTION_DAYS,
    MIN_AUDIT_RETENTION_DAYS, MIN_REQUEST_LOG_RETENTION_DAYS, Settings,
};

static RUNTIME: OnceLock<RwLock<Settings>> = OnceLock::new();

fn cell() -> &'static RwLock<Settings> {
    RUNTIME.get_or_init(|| RwLock::new(Settings::defaults()))
}

/// 当前运行期设置（无锁快照；热路径使用）。
pub fn current() -> Settings {
    match cell().read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// 覆盖运行期设置（启动加载 / PATCH 成功后调用）。
pub fn set(settings: Settings) {
    match cell().write() {
        Ok(mut guard) => *guard = settings,
        Err(poisoned) => *poisoned.into_inner() = settings,
    }
}

/// DB 不可用/缺行时的 env 兜底：保留天数取 env（Config 已校验范围），超时用默认 60。
pub fn fallback_from_config(config: &Config) -> Settings {
    Settings {
        default_provider_id: None,
        request_log_retention_days: clamp_i64(
            config.request_log_retention_days,
            MIN_REQUEST_LOG_RETENTION_DAYS,
            MAX_REQUEST_LOG_RETENTION_DAYS,
            DEFAULT_REQUEST_LOG_RETENTION_DAYS,
        ),
        audit_retention_days: clamp_i64(
            config.audit_retention_days,
            MIN_AUDIT_RETENTION_DAYS,
            MAX_AUDIT_RETENTION_DAYS,
            DEFAULT_AUDIT_RETENTION_DAYS,
        ),
        upstream_timeout_secs: DEFAULT_UPSTREAM_TIMEOUT_SECS,
        updated_at: Utc::now(),
    }
}

fn clamp_i64(value: i64, min: i32, max: i32, default: i32) -> i32 {
    i32::try_from(value)
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}
