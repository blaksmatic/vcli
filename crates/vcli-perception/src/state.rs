//! Cross-tick perception state. Distinct from `PredicateCache` which is
//! per-tick. `PerceptionState` carries:
//!
//! - prior-frame snapshots keyed by predicate hash, for `pixel_diff`
//! - first-true timestamps keyed by (program_id, predicate_name) pair,
//!   for `elapsed_ms_since_true`
//!
//! Interior mutability only — every public method takes `&self`.

use std::sync::Arc;

use dashmap::DashMap;

use vcli_core::clock::UnixMs;
use vcli_core::{PredicateHash, ProgramId};

/// A small perceptual summary of a region from a prior tick. The actual
/// representation is a `Vec<u8>` of 64-bit dHash bytes (8 bytes per
/// snapshot), not the raw pixels — we never store frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorSnapshot {
    /// 64-bit dHash, big-endian.
    pub dhash: u64,
    /// Tick at which this snapshot was recorded.
    pub at_ms: UnixMs,
}

/// Key for per-program-predicate transition tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProgramPredKey {
    /// Program that owns this transition timer.
    pub program: ProgramId,
    /// Predicate name within that program.
    pub predicate: String,
}

/// Cross-tick state shared across evaluators.
#[derive(Debug, Default)]
pub struct PerceptionState {
    /// Prior-frame dHashes by predicate hash (global — same cache key as
    /// the result cache, so cross-program dedup applies).
    prior_snapshots: DashMap<PredicateHash, PriorSnapshot>,
    /// First-true timestamps for `elapsed_ms_since_true`. Program-local
    /// per spec §"`elapsed_ms_since_true`".
    first_true_at: DashMap<ProgramPredKey, UnixMs>,
}

impl PerceptionState {
    /// Fresh empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the most recent snapshot for a pixel_diff predicate hash.
    #[must_use]
    pub fn prior_snapshot(&self, hash: &PredicateHash) -> Option<PriorSnapshot> {
        self.prior_snapshots.get(hash).map(|s| s.clone())
    }

    /// Record a new snapshot. Overwrites any existing entry for this key.
    pub fn record_snapshot(&self, hash: PredicateHash, snapshot: PriorSnapshot) {
        self.prior_snapshots.insert(hash, snapshot);
    }

    /// Read the first-true timestamp for a (program, predicate) pair, if
    /// the child predicate has been truthy on a prior tick without a
    /// falling edge in between.
    #[must_use]
    pub fn first_true_at(&self, key: &ProgramPredKey) -> Option<UnixMs> {
        self.first_true_at.get(key).map(|v| *v)
    }

    /// Record the edge into `true`. If the child was already true, this
    /// leaves the existing timestamp alone (caller should only call on a
    /// rising edge).
    pub fn set_first_true_at(&self, key: ProgramPredKey, at_ms: UnixMs) {
        self.first_true_at.entry(key).or_insert(at_ms);
    }

    /// Clear the first-true timestamp on a falling edge.
    pub fn clear_first_true_at(&self, key: &ProgramPredKey) {
        self.first_true_at.remove(key);
    }

    /// Convenience for the runtime to package `Arc<Self>` into `EvalCtx`.
    #[must_use]
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vcli_core::predicate_hash;

    fn hash(tag: &str) -> PredicateHash {
        predicate_hash(&json!({"tag": tag})).unwrap()
    }

    fn ppk(program_id_str: &str, name: &str) -> ProgramPredKey {
        ProgramPredKey {
            program: program_id_str.parse().unwrap(),
            predicate: name.into(),
        }
    }

    #[test]
    fn prior_snapshot_round_trip() {
        let s = PerceptionState::new();
        let h = hash("a");
        assert!(s.prior_snapshot(&h).is_none());
        s.record_snapshot(
            h.clone(),
            PriorSnapshot {
                dhash: 0xDEAD_BEEF,
                at_ms: 1000,
            },
        );
        assert_eq!(
            s.prior_snapshot(&h),
            Some(PriorSnapshot {
                dhash: 0xDEAD_BEEF,
                at_ms: 1000,
            })
        );
    }

    #[test]
    fn record_snapshot_overwrites() {
        let s = PerceptionState::new();
        let h = hash("a");
        s.record_snapshot(
            h.clone(),
            PriorSnapshot {
                dhash: 1,
                at_ms: 100,
            },
        );
        s.record_snapshot(
            h.clone(),
            PriorSnapshot {
                dhash: 2,
                at_ms: 200,
            },
        );
        assert_eq!(s.prior_snapshot(&h).unwrap().dhash, 2);
    }

    #[test]
    fn first_true_set_then_read() {
        let s = PerceptionState::new();
        let k = ppk("00000000-0000-4000-8000-000000000001", "p");
        assert!(s.first_true_at(&k).is_none());
        s.set_first_true_at(k.clone(), 5000);
        assert_eq!(s.first_true_at(&k), Some(5000));
    }

    #[test]
    fn first_true_set_preserves_earliest() {
        let s = PerceptionState::new();
        let k = ppk("00000000-0000-4000-8000-000000000001", "p");
        s.set_first_true_at(k.clone(), 5000);
        s.set_first_true_at(k.clone(), 9000);
        assert_eq!(s.first_true_at(&k), Some(5000));
    }

    #[test]
    fn first_true_cleared_on_falling_edge() {
        let s = PerceptionState::new();
        let k = ppk("00000000-0000-4000-8000-000000000001", "p");
        s.set_first_true_at(k.clone(), 5000);
        s.clear_first_true_at(&k);
        assert!(s.first_true_at(&k).is_none());
    }

    #[test]
    fn different_programs_do_not_share_first_true_timestamps() {
        let s = PerceptionState::new();
        let a = ppk("00000000-0000-4000-8000-000000000001", "p");
        let b = ppk("00000000-0000-4000-8000-000000000002", "p");
        s.set_first_true_at(a.clone(), 1000);
        assert!(s.first_true_at(&b).is_none());
    }
}
