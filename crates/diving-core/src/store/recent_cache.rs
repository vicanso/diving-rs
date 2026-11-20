//! Small in-memory cache with a fixed time-to-live.
//!
//! Web mode keeps the last few finished analyses here so a burst of
//! requests for one image — the tool calls of an MCP session, a page
//! reload — is served from memory instead of re-validating against the
//! registry and re-parsing the on-disk analysis cache each time.
//!
//! Time is passed in rather than read, so expiry is testable without sleeps.

use lru::LruCache;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

pub struct RecentCache<V> {
    entries: LruCache<String, (V, Instant)>,
    ttl: Duration,
}

impl<V: Clone> RecentCache<V> {
    pub fn new(capacity: NonZeroUsize, ttl: Duration) -> Self {
        Self {
            entries: LruCache::new(capacity),
            ttl,
        }
    }

    fn is_fresh(&self, stored_at: Instant, now: Instant) -> bool {
        now.saturating_duration_since(stored_at) < self.ttl
    }

    /// The value stored under `key`, unless it has outlived the TTL (in
    /// which case it is dropped).
    pub fn get(&mut self, key: &str, now: Instant) -> Option<V> {
        let stored_at = self.entries.peek(key)?.1;
        if !self.is_fresh(stored_at, now) {
            self.entries.pop(key);
            return None;
        }
        // `get` (not `peek`) so the hit counts as a use for LRU eviction.
        self.entries.get(key).map(|(value, _)| value.clone())
    }

    /// Store `value`; the least recently used entry makes room when full.
    pub fn put(&mut self, key: String, value: V, now: Instant) {
        self.entries.put(key, (value, now));
    }

    /// Drop every expired entry, so an idle server does not keep holding
    /// results nobody will ask for again.
    pub fn purge_expired(&mut self, now: Instant) {
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, (_, stored_at))| !self.is_fresh(*stored_at, now))
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.entries.pop(&key);
        }
    }

    /// Drop every entry whose key starts with `prefix` (e.g. when a webhook
    /// notifies that an image tag has been updated).
    pub fn remove_prefix(&mut self, prefix: &str) {
        let matched: Vec<String> = self
            .entries
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, _)| key.clone())
            .collect();
        for key in matched {
            self.entries.pop(&key);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(60);

    fn cache(capacity: usize) -> RecentCache<u32> {
        RecentCache::new(NonZeroUsize::new(capacity).unwrap(), TTL)
    }

    #[test]
    fn serves_entries_until_the_ttl_elapses() {
        let t0 = Instant::now();
        let mut c = cache(4);
        c.put("redis".to_string(), 1, t0);

        assert_eq!(c.get("redis", t0), Some(1));
        assert_eq!(c.get("redis", t0 + TTL - Duration::from_secs(1)), Some(1));
        assert_eq!(c.get("nginx", t0), None);
        // At the TTL the entry is gone — and removed, not just hidden.
        assert_eq!(c.get("redis", t0 + TTL), None);
        assert!(c.is_empty());
    }

    #[test]
    fn storing_again_restarts_the_clock() {
        let t0 = Instant::now();
        let mut c = cache(4);
        c.put("redis".to_string(), 1, t0);
        c.put("redis".to_string(), 2, t0 + Duration::from_secs(50));
        assert_eq!(c.get("redis", t0 + Duration::from_secs(100)), Some(2));
    }

    #[test]
    fn evicts_the_least_recently_used_entry_when_full() {
        let t0 = Instant::now();
        let mut c = cache(2);
        c.put("a".to_string(), 1, t0);
        c.put("b".to_string(), 2, t0);
        assert_eq!(c.get("a", t0), Some(1)); // "b" is now the oldest
        c.put("c".to_string(), 3, t0);

        assert_eq!(c.get("b", t0), None);
        assert_eq!(c.get("a", t0), Some(1));
        assert_eq!(c.get("c", t0), Some(3));
    }

    #[test]
    fn purge_drops_only_expired_entries() {
        let t0 = Instant::now();
        let mut c = cache(4);
        c.put("old".to_string(), 1, t0);
        c.put("new".to_string(), 2, t0 + Duration::from_secs(30));

        c.purge_expired(t0 + TTL);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("new", t0 + TTL), Some(2));
    }

    #[test]
    fn remove_prefix_drops_matching_keys_only() {
        let t0 = Instant::now();
        let mut c = cache(4);
        c.put("hub|org|app|latest|amd64|true".to_string(), 1, t0);
        c.put("hub|org|app|latest|arm64|true".to_string(), 2, t0);
        c.put("hub|org|app|v1|amd64|true".to_string(), 3, t0);

        c.remove_prefix("hub|org|app|latest|");
        assert_eq!(c.get("hub|org|app|latest|amd64|true", t0), None);
        assert_eq!(c.get("hub|org|app|latest|arm64|true", t0), None);
        assert_eq!(c.get("hub|org|app|v1|amd64|true", t0), Some(3));
    }
}
