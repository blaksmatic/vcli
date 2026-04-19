//! Cross-program predicate deduplication.

use std::collections::{BTreeMap, HashSet};

use vcli_core::canonical::{predicate_hash, PredicateHash};
use vcli_core::Predicate;

use crate::error::RuntimeError;

/// One entry in the merged graph.
#[derive(Debug, Clone)]
pub struct DedupEntry<'a> {
    /// Canonical hash — equality implies caller may re-use a prior result.
    pub hash: PredicateHash,
    /// Predicate reference (borrowed from the owning program).
    pub predicate: &'a Predicate,
}

/// Walk a list of programs' active predicate references and return one entry
/// per unique canonical hash. Callers use this to drive a single pass through
/// the `Perception` façade — the per-tick cache inside `Perception` already
/// dedupes at eval time, but this lets the scheduler avoid even building the
/// argument tuple twice.
///
/// # Errors
///
/// Propagates canonicalization errors from `predicate_hash`.
pub fn dedupe<'a, I>(iter: I) -> Result<Vec<DedupEntry<'a>>, RuntimeError>
where
    I: IntoIterator<Item = &'a Predicate>,
{
    let mut seen: HashSet<PredicateHash> = HashSet::new();
    let mut out = Vec::new();
    for p in iter {
        let v = serde_json::to_value(p)
            .map_err(|e| RuntimeError::Internal(format!("serialize pred: {e}")))?;
        let hash = predicate_hash(&v)
            .map_err(|e| RuntimeError::Internal(format!("hash pred: {e}")))?;
        if seen.insert(hash.clone()) {
            out.push(DedupEntry { hash, predicate: p });
        }
    }
    Ok(out)
}

/// Flatten every predicate referenced by a program's watches (by-name) into
/// an iterator of borrowed `Predicate`s. Inline predicates short-circuit the
/// `predicates` lookup.
#[must_use]
pub fn watch_predicates<'a>(
    watches: &'a [vcli_core::Watch],
    predicates: &'a BTreeMap<String, Predicate>,
) -> Vec<&'a Predicate> {
    let mut out = Vec::new();
    for w in watches {
        match &w.when {
            vcli_core::watch::WatchWhen::ByName(n) => {
                if let Some(p) = predicates.get(n) {
                    out.push(p);
                }
            }
            vcli_core::watch::WatchWhen::Inline(p) => {
                out.push(p.as_ref());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::geom::Point;
    use vcli_core::predicate::{PredicateKind, Rgb};
    use vcli_core::watch::{Lifetime, Watch, WatchWhen};

    #[test]
    fn identical_predicates_collapse() {
        let p = PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([1, 2, 3]),
            tolerance: 0,
        };
        let out = dedupe(std::iter::once(&p).chain(std::iter::once(&p))).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn different_predicates_stay_separate() {
        let a = PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([1, 2, 3]),
            tolerance: 0,
        };
        let b = PredicateKind::ColorAt {
            point: Point { x: 1, y: 1 },
            rgb: Rgb([1, 2, 3]),
            tolerance: 0,
        };
        let out = dedupe([&a, &b]).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn watch_predicates_expands_by_name_and_inline() {
        let mut preds = BTreeMap::new();
        preds.insert(
            "red".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let watches = vec![
            Watch {
                when: WatchWhen::ByName("red".into()),
                steps: vec![],
                throttle_ms: 0,
                lifetime: Lifetime::Persistent,
            },
            Watch {
                when: WatchWhen::Inline(Box::new(PredicateKind::ColorAt {
                    point: Point { x: 2, y: 2 },
                    rgb: Rgb([0, 255, 0]),
                    tolerance: 0,
                })),
                steps: vec![],
                throttle_ms: 0,
                lifetime: Lifetime::Persistent,
            },
        ];
        let collected = watch_predicates(&watches, &preds);
        assert_eq!(collected.len(), 2);
    }
}
