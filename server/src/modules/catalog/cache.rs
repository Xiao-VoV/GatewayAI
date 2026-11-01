//! catalog 进程内快照缓存：投影（客户端目录/授权/工具默认/预览）与 routing 视图。
//!
//! 语义：只缓存**配置**（不缓存 subject 授权结果）；`catalog_version` 与投影同快照，
//! 写事务提交后 `invalidate()`，同实例立即生效；跨实例最坏 TTL（5s，单实例部署实际立即）。
//! 不持久化：进程重启重建。

use std::{
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// 快照 TTL（与设计 ≤5s 生效目标一致）。
pub(crate) const SNAPSHOT_TTL: Duration = Duration::from_secs(5);

/// 默认 TTL：生产 = 5s。
///
/// 测试构建（`cfg(test)`）默认 **0s（等价关闭缓存）**：gated 测试会直接 SQL 造数/清表且分布在
/// 其他模块（proxy/routing）的测试里，无法逐个调用 clear；关闭默认 TTL 可避免跨用例脏读。
/// catalog 缓存自测通过 [`set_snapshot_ttl_for_test`] 显式启用真实 TTL。
#[cfg(test)]
pub(crate) const DEFAULT_TTL: Duration = Duration::ZERO;
#[cfg(not(test))]
pub(crate) const DEFAULT_TTL: Duration = SNAPSHOT_TTL;

pub(crate) struct Cached<T> {
    pub(crate) built_at: Instant,
    pub(crate) value: Arc<T>,
}

/// 带 TTL 的单值缓存；读多写少，miss 时并发重复构建可接受（结果一致）。
pub(crate) struct Cache<T> {
    ttl_secs: AtomicU64,
    cell: OnceLock<RwLock<Option<Cached<T>>>>,
}

impl<T> Cache<T> {
    pub(crate) const fn new(ttl: Duration) -> Self {
        Self {
            ttl_secs: AtomicU64::new(ttl.as_secs()),
            cell: OnceLock::new(),
        }
    }

    pub(crate) fn ttl(&self) -> Duration {
        Duration::from_secs(self.ttl_secs.load(Ordering::Relaxed))
    }

    /// 测试专用：调整 TTL（缓存自测显式启用 5s；其余测试保持 0）。
    #[cfg(test)]
    pub(crate) fn set_ttl_for_test(&self, ttl: Duration) {
        self.ttl_secs.store(ttl.as_secs(), Ordering::Relaxed);
    }

    fn cell(&self) -> &RwLock<Option<Cached<T>>> {
        self.cell.get_or_init(|| RwLock::new(None))
    }

    fn read_cell(&self) -> std::sync::RwLockReadGuard<'_, Option<Cached<T>>> {
        match self.cell().read() {
            Ok(guard) => guard,
            // 缓存锁中毒不阻断业务：按 miss 处理（恢复为写锁时重建）。
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_cell(&self) -> std::sync::RwLockWriteGuard<'_, Option<Cached<T>>> {
        match self.cell().write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// 返回未过期的缓存值；过期/未构建返回 `None`。
    pub(crate) fn get(&self) -> Option<Arc<T>> {
        self.get_at(Instant::now())
    }

    pub(crate) fn get_at(&self, now: Instant) -> Option<Arc<T>> {
        let guard = self.read_cell();
        guard.as_ref().and_then(|cached| {
            if is_fresh(cached.built_at, now, self.ttl()) {
                Some(cached.value.clone())
            } else {
                None
            }
        })
    }

    pub(crate) fn store(&self, value: Arc<T>) {
        self.store_at(value, Instant::now());
    }

    pub(crate) fn store_at(&self, value: Arc<T>, now: Instant) {
        *self.write_cell() = Some(Cached {
            built_at: now,
            value,
        });
    }

    /// 清空缓存；下一次读重建。
    pub(crate) fn invalidate(&self) {
        *self.write_cell() = None;
    }

    /// 测试隔离：清空缓存（TTL 由调用方另行恢复）。
    #[cfg(test)]
    pub(crate) fn clear(&self) {
        self.invalidate();
    }
}

/// 缓存是否仍在 TTL 内（`built_at` 晚于 `now` 时视为新鲜，容忍时钟回拨测试注入）。
pub(crate) fn is_fresh(built_at: Instant, now: Instant, ttl: Duration) -> bool {
    match now.checked_duration_since(built_at) {
        Some(elapsed) => elapsed < ttl,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_boundaries() {
        let now = Instant::now();
        assert!(is_fresh(now - Duration::from_secs(4), now, SNAPSHOT_TTL));
        // 恰好等于 TTL 视为过期（严格小于）。
        assert!(!is_fresh(now - SNAPSHOT_TTL, now, SNAPSHOT_TTL));
        assert!(!is_fresh(now - Duration::from_secs(6), now, SNAPSHOT_TTL));
        // 时钟回拨/测试注入：built_at 晚于 now 时按新鲜处理。
        assert!(is_fresh(now + Duration::from_millis(10), now, SNAPSHOT_TTL));
    }

    #[test]
    fn cache_hit_miss_and_invalidate() {
        let cache: Cache<u32> = Cache::new(Duration::from_secs(5));
        assert!(cache.get().is_none());

        cache.store(Arc::new(1));
        assert_eq!(*cache.get().expect("fresh entry"), 1);

        cache.invalidate();
        assert!(cache.get().is_none());

        // TTL=0：写完立即过期。
        let no_ttl: Cache<u32> = Cache::new(Duration::ZERO);
        no_ttl.store(Arc::new(7));
        assert!(no_ttl.get().is_none());

        // 注入时间：TTL 边界前后。
        let injected: Cache<u32> = Cache::new(Duration::from_secs(5));
        let built = Instant::now();
        injected.store_at(Arc::new(9), built);
        assert_eq!(
            *injected
                .get_at(built + Duration::from_secs(4))
                .expect("fresh"),
            9
        );
        assert!(injected.get_at(built + Duration::from_secs(5)).is_none());
    }

    #[test]
    fn cache_clear_is_idempotent() {
        let cache: Cache<String> = Cache::new(Duration::from_secs(5));
        cache.store(Arc::new("v".to_string()));
        cache.clear();
        cache.clear();
        assert!(cache.get().is_none());
    }
}
