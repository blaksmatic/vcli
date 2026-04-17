//! Perception error type. Evaluators surface a fast-path `PredicateResult`
//! for "predicate is false because inputs disagree" (not an error), and
//! return `PerceptionError` only for conditions the runtime must handle
//! (bad asset bytes, out-of-bounds region, unknown referenced predicate).

use thiserror::Error;

/// Alias for `Result<T, PerceptionError>`.
pub type Result<T> = std::result::Result<T, PerceptionError>;

/// Errors surfaced by evaluators.
#[derive(Debug, Error)]
pub enum PerceptionError {
    /// A logical predicate referenced a name not present in the program.
    #[error("unknown predicate reference: {0}")]
    UnknownPredicate(String),
    /// A predicate referenced itself directly or transitively. The DSL
    /// validator should catch this at submit; the evaluator double-checks.
    #[error("cycle detected at predicate: {0}")]
    Cycle(String),
    /// Asset bytes could not be decoded.
    #[error("asset decode: {0}")]
    AssetDecode(String),
    /// Region is outside the frame bounds (partial overlap is clipped, but
    /// zero overlap is reported).
    #[error("region outside frame bounds")]
    RegionOutOfBounds,
    /// A `sha256:<hex>` asset reference was passed to an evaluator without
    /// a resolver having first attached the raw bytes. Bug, not user error.
    #[error("asset bytes not materialized for reference: {0}")]
    AssetNotMaterialized(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_unknown_predicate() {
        let e = PerceptionError::UnknownPredicate("foo".into());
        assert_eq!(e.to_string(), "unknown predicate reference: foo");
    }

    #[test]
    fn display_cycle() {
        let e = PerceptionError::Cycle("a".into());
        assert_eq!(e.to_string(), "cycle detected at predicate: a");
    }

    #[test]
    fn display_asset_decode() {
        let e = PerceptionError::AssetDecode("bad PNG".into());
        assert!(e.to_string().contains("bad PNG"));
    }
}
