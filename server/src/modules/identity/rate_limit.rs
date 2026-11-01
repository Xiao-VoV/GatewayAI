//! 登录/setup-token 失败限流（D19）。
//!
//! - `(IP, email)`：15 分钟滑动窗口内 5 次失败
//! - `IP`：15 分钟滑动窗口内 20 次失败（撞库兜底）
//! - `IP`（setup token 错误）：15 分钟 5 次
//! - 进程内、有界容量（上限 10k 键，按最近使用淘汰最旧约 10%）
//! - `Retry-After` = 剩余窗口秒数（最小 60）

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(15 * 60);
const EMAIL_LIMIT: usize = 5;
const IP_LIMIT: usize = 20;
const SETUP_IP_LIMIT: usize = 5;
const MAX_KEYS: usize = 10_000;
const MIN_RETRY_AFTER_SECS: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Email { ip: String, email: String },
    Ip(String),
    SetupIp(String),
}

#[derive(Debug)]
struct Entry {
    failures: Vec<Instant>,
    last_seen: Instant,
    seq: u64,
}

#[derive(Debug)]
pub struct RateLimiter {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    entries: HashMap<Key, Entry>,
    next_seq: u64,
}

impl Inner {
    fn check(&self, keys: &[(Key, usize)], now: Instant) -> Option<u64> {
        let mut retry = 0u64;
        for (key, limit) in keys {
            if let Some(entry) = self.entries.get(key) {
                let mut entry = Entry {
                    failures: entry.failures.clone(),
                    last_seen: entry.last_seen,
                    seq: entry.seq,
                };
                prune(&mut entry, now);
                if entry.failures.len() >= *limit
                    && let Some(oldest) = entry.failures.first()
                {
                    let remaining = WINDOW.saturating_sub(now.duration_since(*oldest));
                    retry = retry.max(remaining.as_secs().max(MIN_RETRY_AFTER_SECS));
                }
            }
        }
        (retry > 0).then_some(retry)
    }

    fn record(&mut self, keys: &[Key], now: Instant) {
        self.next_seq += 1;
        let seq = self.next_seq;
        for key in keys {
            let entry = self.entries.entry(key.clone()).or_insert_with(|| Entry {
                failures: Vec::new(),
                last_seen: now,
                seq,
            });
            prune(entry, now);
            entry.failures.push(now);
            entry.last_seen = now;
            entry.seq = seq;
        }
        self.evict_if_needed();
    }

    fn evict_if_needed(&mut self) {
        if self.entries.len() <= MAX_KEYS {
            return;
        }
        // 按最近使用时间淘汰最旧约 10%（避免每次插入 O(n)）。
        let target = MAX_KEYS - MAX_KEYS / 10;
        let mut by_last_seen: Vec<(Instant, Key)> = self
            .entries
            .iter()
            .map(|(key, entry)| (entry.last_seen, key.clone()))
            .collect();
        by_last_seen.sort_by_key(|(last_seen, _)| *last_seen);
        let remove_count = self.entries.len().saturating_sub(target);
        for (_, key) in by_last_seen.into_iter().take(remove_count) {
            self.entries.remove(&key);
        }
    }

    fn clear(&mut self, keys: &[Key]) {
        for key in keys {
            self.entries.remove(key);
        }
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn check_login(&self, ip: &str, email: &str, now: Instant) -> Option<u64> {
        let inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.check(
            &[
                (
                    Key::Email {
                        ip: ip.to_string(),
                        email: email.to_string(),
                    },
                    EMAIL_LIMIT,
                ),
                (Key::Ip(ip.to_string()), IP_LIMIT),
            ],
            now,
        )
    }

    pub fn record_login_failure(&self, ip: &str, email: &str, now: Instant) {
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.record(
            &[
                Key::Email {
                    ip: ip.to_string(),
                    email: email.to_string(),
                },
                Key::Ip(ip.to_string()),
            ],
            now,
        );
    }

    pub fn clear_login(&self, ip: &str, email: &str) {
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.clear(&[
            Key::Email {
                ip: ip.to_string(),
                email: email.to_string(),
            },
            Key::Ip(ip.to_string()),
        ]);
    }

    pub fn check_setup(&self, ip: &str, now: Instant) -> Option<u64> {
        let inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.check(&[(Key::SetupIp(ip.to_string()), SETUP_IP_LIMIT)], now)
    }

    pub fn record_setup_failure(&self, ip: &str, now: Instant) {
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.record(&[Key::SetupIp(ip.to_string())], now);
    }

    pub fn clear_setup(&self, ip: &str) {
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.clear(&[Key::SetupIp(ip.to_string())]);
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程级限流器（S1 单实例；D19）。
static GLOBAL: OnceLock<RateLimiter> = OnceLock::new();

pub fn global() -> &'static RateLimiter {
    GLOBAL.get_or_init(RateLimiter::new)
}

fn prune(entry: &mut Entry, now: Instant) {
    entry.failures.retain(|at| now.duration_since(*at) < WINDOW);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn email_threshold_triggers_after_five_failures() {
        let limiter = RateLimiter::new();
        let now = t0();
        for _ in 0..5 {
            assert!(
                limiter
                    .check_login("1.2.3.4", "a@corp.example.com", now)
                    .is_none()
            );
            limiter.record_login_failure("1.2.3.4", "a@corp.example.com", now);
        }
        let retry = limiter
            .check_login("1.2.3.4", "a@corp.example.com", now)
            .expect("limited");
        assert!(retry >= 60);
    }

    #[test]
    fn different_emails_share_ip_backstop() {
        let limiter = RateLimiter::new();
        let now = t0();
        for index in 0..20 {
            let email = format!("user{index}@corp.example.com");
            assert!(limiter.check_login("5.6.7.8", &email, now).is_none());
            limiter.record_login_failure("5.6.7.8", &email, now);
        }
        let retry = limiter.check_login("5.6.7.8", "fresh@corp.example.com", now);
        assert!(retry.is_some(), "per-IP backstop must trigger");
    }

    #[test]
    fn window_expiry_releases_limit() {
        let limiter = RateLimiter::new();
        let now = t0();
        for _ in 0..5 {
            limiter.record_login_failure("9.9.9.9", "b@corp.example.com", now);
        }
        assert!(
            limiter
                .check_login("9.9.9.9", "b@corp.example.com", now)
                .is_some()
        );
        let later = now + WINDOW + Duration::from_secs(1);
        assert!(
            limiter
                .check_login("9.9.9.9", "b@corp.example.com", later)
                .is_none()
        );
    }

    #[test]
    fn success_clears_counters() {
        let limiter = RateLimiter::new();
        let now = t0();
        for _ in 0..5 {
            limiter.record_login_failure("8.8.8.8", "c@corp.example.com", now);
        }
        limiter.clear_login("8.8.8.8", "c@corp.example.com");
        assert!(
            limiter
                .check_login("8.8.8.8", "c@corp.example.com", now)
                .is_none()
        );
    }

    #[test]
    fn setup_token_limit_is_separate() {
        let limiter = RateLimiter::new();
        let now = t0();
        for _ in 0..5 {
            assert!(limiter.check_setup("7.7.7.7", now).is_none());
            limiter.record_setup_failure("7.7.7.7", now);
        }
        assert!(limiter.check_setup("7.7.7.7", now).is_some());
        // 登录计数不受 setup 键影响
        assert!(
            limiter
                .check_login("7.7.7.7", "d@corp.example.com", now)
                .is_none()
        );
    }
}
