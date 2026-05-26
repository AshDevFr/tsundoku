//! In-memory negative cache for "known-miss" lookups.
//!
//! `(provider, external_id)` pairs that resolved to `Ok(None)` from the API
//! are recorded here so subsequent calls within `ttl` skip the network.
//! The cache is bounded: when `max_entries` is reached we drop the oldest
//! half rather than wandering off into unbounded growth.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

pub struct NegativeCache {
    inner: RwLock<HashMap<(String, String), Instant>>,
    ttl: Duration,
    max_entries: usize,
}

impl NegativeCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            ttl,
            max_entries,
        }
    }

    /// `true` if a non-expired miss is recorded for `(provider, id)`.
    /// Returns `false` if missing or expired (and removes the expired entry).
    pub async fn is_known_miss(&self, provider: &str, id: &str) -> bool {
        let key = (provider.to_string(), id.to_string());
        {
            let guard = self.inner.read().await;
            if let Some(recorded_at) = guard.get(&key)
                && recorded_at.elapsed() < self.ttl
            {
                return true;
            }
        }
        // Either no entry or it's stale; drop the stale one if present.
        let mut guard = self.inner.write().await;
        if let Some(recorded_at) = guard.get(&key)
            && recorded_at.elapsed() >= self.ttl
        {
            guard.remove(&key);
        }
        false
    }

    /// Record a miss. If we'd exceed `max_entries`, evict the oldest half.
    pub async fn record_miss(&self, provider: &str, id: &str) {
        let mut guard = self.inner.write().await;
        if guard.len() >= self.max_entries {
            // Simple bounded eviction: keep the newest half. Costs O(n log n)
            // but only runs at the cap boundary; negligible at v1 scale.
            let mut entries: Vec<((String, String), Instant)> = guard.drain().collect();
            entries.sort_by_key(|(_, t)| std::cmp::Reverse(*t));
            entries.truncate(self.max_entries / 2);
            for (k, v) in entries {
                guard.insert(k, v);
            }
        }
        guard.insert((provider.to_string(), id.to_string()), Instant::now());
    }

    /// Remove an entry, e.g. after a successful refresh discovers a series
    /// that was previously cached as a miss.
    pub async fn forget(&self, provider: &str, id: &str) {
        let mut guard = self.inner.write().await;
        guard.remove(&(provider.to_string(), id.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn records_and_returns_hit() {
        let cache = NegativeCache::new(Duration::from_secs(60), 100);
        assert!(!cache.is_known_miss("mangabaka", "1").await);
        cache.record_miss("mangabaka", "1").await;
        assert!(cache.is_known_miss("mangabaka", "1").await);
    }

    #[tokio::test]
    async fn expired_entries_are_purged_on_lookup() {
        let cache = NegativeCache::new(Duration::from_millis(20), 100);
        cache.record_miss("mangabaka", "1").await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(!cache.is_known_miss("mangabaka", "1").await);
        // And the underlying entry is gone.
        assert_eq!(cache.inner.read().await.len(), 0);
    }

    #[tokio::test]
    async fn forget_removes_specific_entry() {
        let cache = NegativeCache::new(Duration::from_secs(60), 100);
        cache.record_miss("mangabaka", "1").await;
        cache.record_miss("mangabaka", "2").await;
        cache.forget("mangabaka", "1").await;
        assert!(!cache.is_known_miss("mangabaka", "1").await);
        assert!(cache.is_known_miss("mangabaka", "2").await);
    }

    #[tokio::test]
    async fn bounded_growth_evicts_oldest_half_at_cap() {
        let cache = NegativeCache::new(Duration::from_secs(60), 4);
        cache.record_miss("p", "1").await;
        cache.record_miss("p", "2").await;
        cache.record_miss("p", "3").await;
        cache.record_miss("p", "4").await;
        // The fifth insert triggers eviction down to max/2 = 2 entries.
        cache.record_miss("p", "5").await;
        let len = cache.inner.read().await.len();
        assert!(len <= 4, "len after eviction should be <= max, got {len}");
        assert!(cache.is_known_miss("p", "5").await, "newest entry survives");
    }
}
