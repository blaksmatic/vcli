//! Per-predicate hash computation. For each named predicate, canonicalize its
//! `serde_json` representation (via `vcli_core::canonicalize`) and hash it
//! (`predicate_hash`). Output is keyed by predicate name for easy cache-key
//! lookup downstream.
//!
//! Decision 1.1 + 1.3: the canonical subgraph belongs in the daemon once
//! `relative_to.predicate` and logical dependencies are resolved to hashes,
//! but at DSL-time we still need a stable per-predicate fingerprint — that's
//! what this table is.

use std::collections::BTreeMap;

use vcli_core::{predicate_hash, PredicateHash, PredicateKind};

use crate::error::DslError;
use crate::path::JsonPath;

/// Name-keyed per-predicate hashes.
pub type PredicateHashes = BTreeMap<String, PredicateHash>;

/// Compute one [`PredicateHash`] per predicate in the map by serializing and
/// canonicalizing its JSON form. Errors surface with a path pointing at the
/// offending predicate.
///
/// # Errors
///
/// Returns a [`DslError`] if canonicalization or re-serialization fails (in
/// practice the former can only fail on non-finite floats and the latter is
/// infallible for owned `PredicateKind`).
pub fn compute_predicate_hashes(
    predicates: &BTreeMap<String, PredicateKind>,
) -> Result<PredicateHashes, DslError> {
    let mut out = PredicateHashes::new();
    for (name, pred) in predicates {
        let val = serde_json::to_value(pred).map_err(|e| {
            DslError::new(
                crate::error::DslErrorKind::JsonParse(e.to_string()),
                JsonPath::root().key("predicates").key(name),
            )
        })?;
        let h = predicate_hash(&val).map_err(|e| {
            let mut de: DslError = e.into();
            de.path = JsonPath::root().key("predicates").key(name);
            de
        })?;
        out.insert(name.clone(), h);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn preds(v: serde_json::Value) -> BTreeMap<String, PredicateKind> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn empty_in_empty_out() {
        let h = compute_predicate_hashes(&BTreeMap::new()).unwrap();
        assert!(h.is_empty());
    }

    #[test]
    fn one_hash_per_name() {
        let p = preds(json!({
            "a": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":1},
            "b": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[255,255,255],"tolerance":1}
        }));
        let h = compute_predicate_hashes(&p).unwrap();
        assert_eq!(h.len(), 2);
        assert_ne!(h["a"], h["b"]);
        assert_eq!(h["a"].hex().len(), 64);
    }

    #[test]
    fn structurally_equal_predicates_share_hash() {
        let p = preds(json!({
            "a": {"kind":"not","of":"ref"},
            "b": {"kind":"not","of":"ref"}
        }));
        let h = compute_predicate_hashes(&p).unwrap();
        assert_eq!(h["a"], h["b"]);
    }

    #[test]
    fn hash_stable_under_key_order_within_predicate() {
        // Both maps serialize the same predicate; serde_json::Value normalizes
        // keys alphabetically via BTreeMap, but canonicalize is what guarantees
        // hash stability — assert on concrete equality.
        let p1 = preds(json!({
            "t": {"kind":"template","image":"x.png","confidence":0.9,
                  "region":{"kind":"absolute","box":{"x":0,"y":0,"w":10,"h":10}},
                  "throttle_ms":200}
        }));
        let p2 = preds(json!({
            "t": {"image":"x.png","kind":"template","confidence":0.9,
                  "throttle_ms":200,
                  "region":{"box":{"w":10,"y":0,"x":0,"h":10},"kind":"absolute"}}
        }));
        let h1 = compute_predicate_hashes(&p1).unwrap();
        let h2 = compute_predicate_hashes(&p2).unwrap();
        assert_eq!(h1["t"], h2["t"]);
    }
}
