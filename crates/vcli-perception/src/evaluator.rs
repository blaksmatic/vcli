//! The `Evaluator` trait and its evaluation context.
//!
//! Per Decision A, `evaluate` takes `&self` so multiple evaluators can run
//! concurrently under `rayon::par_iter` with a shared `DashMap` cache.
//! Evaluators that need mutable state (`pixel_diff` prior frames, elapsed
//! first-true timestamps) push that state into `PerceptionState`, which
//! uses interior mutability (`DashMap` / `Mutex`) — never `&mut self`.

use std::collections::BTreeMap;

use vcli_core::clock::UnixMs;
use vcli_core::{Frame, Predicate, PredicateResult};

use crate::cache::PredicateCache;
use crate::error::Result;
use crate::state::PerceptionState;

/// Evaluation context threaded to every `Evaluator::evaluate` call.
///
/// Lifetime: exactly one tick. The cache is per-tick; the state is
/// cross-tick but shared by reference here.
pub struct EvalCtx<'a> {
    /// Captured frame for this tick.
    pub frame: &'a Frame,
    /// Unix-ms timestamp of this tick (from the `Clock`).
    pub now_ms: UnixMs,
    /// Per-tick cache (cleared by the runtime at tick start).
    pub cache: &'a PredicateCache,
    /// Cross-tick state (interior mutability).
    pub state: &'a PerceptionState,
    /// Named predicate graph for this program. Logical and
    /// `elapsed_ms_since_true` evaluators look up dependencies here.
    pub predicates: &'a BTreeMap<String, Predicate>,
    /// Asset bytes, keyed by sha256 hex (no `sha256:` prefix). Populated
    /// by the daemon submit module before handing off to Perception.
    pub assets: &'a BTreeMap<String, Vec<u8>>,
    /// The program currently being evaluated. Only used by predicates with
    /// program-local state (`elapsed_ms_since_true`). `None` when the
    /// runtime evaluates a trigger-independent predicate.
    pub program: Option<vcli_core::ProgramId>,
}

/// A predicate evaluator. One impl per predicate kind.
pub trait Evaluator: Send + Sync {
    /// Evaluate the predicate against the current frame + state.
    ///
    /// # Errors
    ///
    /// Returns `PerceptionError` only for conditions the runtime must
    /// handle (bad asset, region out of bounds, unknown reference).
    /// A "predicate is false because screen doesn't match" is NOT an
    /// error — it returns `Ok(PredicateResult { truthy: false, ... })`.
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time test: a no-op evaluator implements the trait.
    struct NoOp;
    impl Evaluator for NoOp {
        fn evaluate(&self, _p: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
            Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            })
        }
    }

    #[test]
    fn trait_is_object_safe_and_send_sync() {
        // Compiles only if Evaluator is object-safe and Send + Sync.
        let _boxed: Box<dyn Evaluator> = Box::new(NoOp);
    }
}
