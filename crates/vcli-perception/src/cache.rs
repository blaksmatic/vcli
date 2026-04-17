//! Per-tick predicate result cache. Decision A — `DashMap` for lock-free
//! reads and sharded writes. The runtime calls `clear()` at the start of
//! every tick so evaluators within a tick see each other's results but
//! no stale results carry across ticks.
//!
//! Program-local temporal predicates (`elapsed_ms_since_true`) keep their
//! state in `PerceptionState`, not here.

use dashmap::DashMap;
use vcli_core::{PredicateHash, PredicateResult};

/// The shared per-tick cache.
#[derive(Debug, Default)]
pub struct PredicateCache {
    entries: DashMap<PredicateHash, PredicateResult>,
}

impl PredicateCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a cached result if present.
    #[must_use]
    pub fn get(&self, hash: &PredicateHash) -> Option<PredicateResult> {
        self.entries.get(hash).map(|r| r.clone())
    }

    /// Store a result. If an entry already exists for this hash, it is
    /// overwritten (this is the cheap retry path — it should be rare
    /// because evaluators check `get` first).
    pub fn insert(&self, hash: PredicateHash, result: PredicateResult) {
        self.entries.insert(hash, result);
    }

    /// Number of entries. Diagnostic use only.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Invalidate all entries. Called by the runtime at tick start.
    pub fn clear(&self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vcli_core::predicate_hash;

    fn fake_result(truthy: bool) -> PredicateResult {
        PredicateResult {
            truthy,
            match_data: None,
            at: 1000,
        }
    }

    fn some_hash(tag: &str) -> PredicateHash {
        predicate_hash(&json!({"kind": "color_at", "tag": tag})).unwrap()
    }

    #[test]
    fn get_miss_returns_none() {
        let c = PredicateCache::new();
        assert!(c.get(&some_hash("a")).is_none());
        assert!(c.is_empty());
    }

    #[test]
    fn insert_then_get_returns_result() {
        let c = PredicateCache::new();
        let h = some_hash("a");
        c.insert(h.clone(), fake_result(true));
        assert_eq!(c.get(&h), Some(fake_result(true)));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn insert_overwrites_existing_entry() {
        let c = PredicateCache::new();
        let h = some_hash("a");
        c.insert(h.clone(), fake_result(false));
        c.insert(h.clone(), fake_result(true));
        assert_eq!(c.get(&h), Some(fake_result(true)));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn clear_wipes_all_entries() {
        let c = PredicateCache::new();
        c.insert(some_hash("a"), fake_result(true));
        c.insert(some_hash("b"), fake_result(false));
        assert_eq!(c.len(), 2);
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn concurrent_inserts_from_many_threads() {
        use std::sync::Arc;
        use std::thread;

        let c = Arc::new(PredicateCache::new());
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let c = Arc::clone(&c);
                thread::spawn(move || {
                    let tag = format!("k{i}");
                    c.insert(some_hash(&tag), fake_result(i % 2 == 0));
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(c.len(), 16);
    }
}
