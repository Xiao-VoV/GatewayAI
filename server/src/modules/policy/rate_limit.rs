//! Key 级 RPM 限流：进程内 60s 滑动窗口（S3.1 / US-011）。
//!
//! 边界：仅 Key RPM；单实例、进程内计数（重启归零；多实例各自计数）；
//! TPM/多层级/分布式/并发数限流后置。配置面在 access（`virtual_keys.rpm_limit`）。
//!
//! 语义（设计 `设计文档（暂未开源）` §3）：
//! - `None`（或非法 `<=0`）= 不限；
//! - 窗口内请求数 < limit → 入队并放行（允许 limit 个并发请求同时通过）；
//! - 否则拒绝，`retry_after` = ceil(60s - 最早请求已过时间)，最小 1 秒；
//! - 被拒绝的请求不占名额；不写 DB、不阻塞。

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use uuid::Uuid;

/// 滑动窗口长度：60 秒。
pub const WINDOW: Duration = Duration::from_secs(60);
const RETRY_AFTER_MAX_SECS: u64 = 60;
/// 每 N 次检查做一次全表过期清理，避免长期空 key 占用内存。
const SWEEP_EVERY: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateDecision {
    pub allowed: bool,
    pub retry_after_secs: u64,
}

impl RateDecision {
    fn allowed() -> Self {
        Self {
            allowed: true,
            retry_after_secs: 0,
        }
    }

    fn denied(retry_after_secs: u64) -> Self {
        Self {
            allowed: false,
            retry_after_secs: retry_after_secs.clamp(1, RETRY_AFTER_MAX_SECS),
        }
    }
}

type Windows = HashMap<Uuid, VecDeque<Instant>>;

fn windows() -> &'static Mutex<Windows> {
    static WINDOWS: OnceLock<Mutex<Windows>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn checks() -> &'static AtomicU64 {
    static CHECKS: AtomicU64 = AtomicU64::new(0);
    &CHECKS
}

/// 生产入口：当前时刻检查。
pub fn check_key_rpm(key_id: Uuid, rpm_limit: Option<i32>) -> RateDecision {
    check_key_rpm_at(key_id, rpm_limit, Instant::now())
}

/// 可注入时钟的检查（单测/可控窗口），生产走 [`check_key_rpm`]。
pub fn check_key_rpm_at(key_id: Uuid, rpm_limit: Option<i32>, now: Instant) -> RateDecision {
    let Some(limit) = rpm_limit.filter(|limit| *limit > 0) else {
        return RateDecision::allowed();
    };
    let limit = limit as usize;
    let mut windows = match windows().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let queue = windows.entry(key_id).or_default();
    while let Some(front) = queue.front() {
        if now.saturating_duration_since(*front) >= WINDOW {
            queue.pop_front();
        } else {
            break;
        }
    }
    let decision = if queue.len() < limit {
        queue.push_back(now);
        RateDecision::allowed()
    } else {
        let oldest = queue.front().copied().unwrap_or(now);
        let elapsed = now.saturating_duration_since(oldest).as_secs_f64();
        let retry = (WINDOW.as_secs_f64() - elapsed).ceil() as u64;
        RateDecision::denied(retry)
    };
    // 周期清理：删除窗口已空/过期的 key，防止 map 无界增长。
    let count = checks().fetch_add(1, Ordering::Relaxed) + 1;
    if count.is_multiple_of(SWEEP_EVERY) {
        windows.retain(|_, queue| {
            while let Some(front) = queue.front() {
                if now.saturating_duration_since(*front) >= WINDOW {
                    queue.pop_front();
                } else {
                    break;
                }
            }
            !queue.is_empty()
        });
    }
    decision
}

#[cfg(test)]
pub fn reset_for_tests() {
    if let Ok(mut windows) = windows().lock() {
        windows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{MutexGuard, OnceLock};

    /// 全局 limiter 是进程内单例；测试串行化，避免 reset/窗口互相干扰。
    fn serial() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn at(millis: u64) -> Instant {
        Instant::now() + Duration::from_millis(millis)
    }

    #[test]
    fn limit_two_allows_first_two_and_denies_third() {
        let _guard = serial();
        reset_for_tests();
        let key = Uuid::new_v4();
        let now = at(0);
        assert!(check_key_rpm_at(key, Some(2), now).allowed);
        assert!(check_key_rpm_at(key, Some(2), now).allowed);
        let denied = check_key_rpm_at(key, Some(2), now);
        assert!(!denied.allowed);
        assert!((1..=60).contains(&denied.retry_after_secs));
    }

    #[test]
    fn retry_after_counts_down_then_window_recovers() {
        let _guard = serial();
        reset_for_tests();
        let key = Uuid::new_v4();
        let now = at(0);
        assert!(check_key_rpm_at(key, Some(1), now).allowed);
        let denied = check_key_rpm_at(key, Some(1), now + Duration::from_secs(30));
        assert!(!denied.allowed);
        assert_eq!(denied.retry_after_secs, 30);
        // 超过窗口后旧记录过期，恢复放行。
        assert!(check_key_rpm_at(key, Some(1), now + WINDOW + Duration::from_millis(1)).allowed);
    }

    #[test]
    fn none_limit_is_unlimited_and_does_not_occupy_window() {
        let _guard = serial();
        reset_for_tests();
        let key = Uuid::new_v4();
        let now = at(0);
        for _ in 0..100 {
            assert!(check_key_rpm_at(key, None, now).allowed);
        }
        // None 不计数：切到 limit=1 时仍可放行一次。
        assert!(check_key_rpm_at(key, Some(1), now).allowed);
        assert!(!check_key_rpm_at(key, Some(1), now).allowed);
    }

    #[test]
    fn keys_are_isolated() {
        let _guard = serial();
        reset_for_tests();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let now = at(0);
        assert!(check_key_rpm_at(a, Some(1), now).allowed);
        assert!(!check_key_rpm_at(a, Some(1), now).allowed);
        assert!(check_key_rpm_at(b, Some(1), now).allowed);
    }

    #[test]
    fn denied_requests_do_not_consume_slots() {
        let _guard = serial();
        reset_for_tests();
        let key = Uuid::new_v4();
        let now = at(0);
        assert!(check_key_rpm_at(key, Some(1), now).allowed);
        for _ in 0..5 {
            assert!(!check_key_rpm_at(key, Some(1), now).allowed);
        }
        // 窗口过期后只放行新请求一次（拒绝请求没有入队）。
        assert!(check_key_rpm_at(key, Some(1), now + WINDOW + Duration::from_millis(1)).allowed);
    }

    #[test]
    fn non_positive_limit_is_unlimited_defensively() {
        let _guard = serial();
        reset_for_tests();
        let key = Uuid::new_v4();
        let now = at(0);
        for _ in 0..10 {
            assert!(check_key_rpm_at(key, Some(0), now).allowed);
        }
    }
}
