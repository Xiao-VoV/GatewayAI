//! access 失败限流：配对码/IP、token 交换 (IP, device)/IP。
//!
//! - 配对失败：10 次 / 10 分钟 / IP
//! - token 交换失败：30 次 / 10 分钟 / (IP, device_id)；100 次 / 10 分钟 / IP 兜底
//! - 进程内、有界容量；`Retry-After` = 剩余窗口秒数（最小 60）
//! - S3 单实例；多实例版本见技术设计 §4。

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::model::{
    PAIRING_MAX_FAILURES, PAIRING_WINDOW_SECS, TOKEN_DEVICE_MAX_FAILURES, TOKEN_IP_MAX_FAILURES,
};

const WINDOW: Duration = Duration::from_secs(PAIRING_WINDOW_SECS);
const MAX_KEYS: usize = 10_000;
const MIN_RETRY_AFTER_SECS: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    PairIp(String),
    TokenIpDevice { ip: String, device: String },
    TokenIp(String),
}

#[derive(Debug)]
struct Entry {
    failures: Vec<Instant>,
    last_seen: Instant,
    seq: u64,
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
}

#[derive(Debug)]
pub struct AccessRateLimiter {
    inner: Mutex<Inner>,
}

impl AccessRateLimiter {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn check_pair(&self, ip: &str, now: Instant) -> Option<u64> {
        let inner = self.inner.lock().expect("access rate limiter poisoned");
        inner.check(&[(Key::PairIp(ip.to_string()), PAIRING_MAX_FAILURES)], now)
    }

    pub fn record_pair_failure(&self, ip: &str, now: Instant) {
        let mut inner = self.inner.lock().expect("access rate limiter poisoned");
        inner.record(&[Key::PairIp(ip.to_string())], now);
    }

    pub fn check_token(&self, ip: &str, device: &str, now: Instant) -> Option<u64> {
        let inner = self.inner.lock().expect("access rate limiter poisoned");
        inner.check(
            &[
                (
                    Key::TokenIpDevice {
                        ip: ip.to_string(),
                        device: device.to_string(),
                    },
                    TOKEN_DEVICE_MAX_FAILURES,
                ),
                (Key::TokenIp(ip.to_string()), TOKEN_IP_MAX_FAILURES),
            ],
            now,
        )
    }

    pub fn record_token_failure(&self, ip: &str, device: &str, now: Instant) {
        let mut inner = self.inner.lock().expect("access rate limiter poisoned");
        inner.record(
            &[
                Key::TokenIpDevice {
                    ip: ip.to_string(),
                    device: device.to_string(),
                },
                Key::TokenIp(ip.to_string()),
            ],
            now,
        );
    }
}

impl Default for AccessRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL: OnceLock<AccessRateLimiter> = OnceLock::new();

pub fn global() -> &'static AccessRateLimiter {
    GLOBAL.get_or_init(AccessRateLimiter::new)
}

fn prune(entry: &mut Entry, now: Instant) {
    entry.failures.retain(|at| now.duration_since(*at) < WINDOW);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_limiter_blocks_after_limit() {
        let limiter = AccessRateLimiter::new();
        let now = Instant::now();
        for _ in 0..PAIRING_MAX_FAILURES {
            assert!(limiter.check_pair("1.2.3.4", now).is_none());
            limiter.record_pair_failure("1.2.3.4", now);
        }
        assert!(limiter.check_pair("1.2.3.4", now).is_some());
        assert!(limiter.check_pair("9.9.9.9", now).is_none());
    }

    #[test]
    fn token_limiter_is_keyed_by_ip_and_device() {
        let limiter = AccessRateLimiter::new();
        let now = Instant::now();
        for _ in 0..TOKEN_DEVICE_MAX_FAILURES {
            limiter.record_token_failure("1.2.3.4", "device-a", now);
        }
        assert!(limiter.check_token("1.2.3.4", "device-a", now).is_some());
        assert!(limiter.check_token("1.2.3.4", "device-b", now).is_none());
    }
}
