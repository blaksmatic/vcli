//! Public façade. The runtime asks for a named predicate's result; the
//! façade canonicalizes, checks the cache, dispatches to the right
//! evaluator, memoizes the result, and returns it. Cache is wiped at
//! tick boundaries by the runtime calling `PredicateCache::clear()`.

use std::collections::BTreeMap;
use std::sync::Arc;

use vcli_core::clock::UnixMs;
use vcli_core::predicate::PredicateKind;
use vcli_core::{predicate_hash, Frame, Predicate, PredicateHash, PredicateResult, ProgramId};

use crate::cache::PredicateCache;
use crate::color_at::ColorAtEvaluator;
use crate::elapsed::ElapsedMsSinceTrueEvaluator;
use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::logical::{AllOfEvaluator, AnyOfEvaluator, NotEvaluator};
use crate::pixel_diff::PixelDiffEvaluator;
use crate::state::PerceptionState;
use crate::template::TemplateEvaluator;

/// Top-level perception entry point. Holds the per-tick cache and a handle
/// to cross-tick state.
pub struct Perception {
    cache: Arc<PredicateCache>,
    state: Arc<PerceptionState>,
}

impl Perception {
    /// Create a fresh perception with empty cache + state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: Arc::new(PredicateCache::new()),
            state: Arc::new(PerceptionState::new()),
        }
    }

    /// Reuse existing shared state (e.g. after a runtime restart that
    /// wants to preserve first-true timers across a short gap).
    #[must_use]
    pub fn with_state(state: Arc<PerceptionState>) -> Self {
        Self {
            cache: Arc::new(PredicateCache::new()),
            state,
        }
    }

    /// Borrow the cache. The runtime calls `.clear()` at tick start.
    #[must_use]
    pub fn cache(&self) -> &PredicateCache {
        &self.cache
    }

    /// Borrow the cross-tick state. Diagnostic use only.
    #[must_use]
    pub fn state(&self) -> &PerceptionState {
        &self.state
    }

    /// Wipe the per-tick cache. Called by the runtime at the start of
    /// every tick. Cross-tick state is preserved.
    pub fn clear(&self) {
        self.cache.clear();
    }

    /// Evaluate a named predicate, memoizing through the shared cache.
    ///
    /// # Errors
    ///
    /// Propagates evaluator errors — see `PerceptionError`.
    pub fn evaluate_named(
        &self,
        name: &str,
        predicates: &BTreeMap<String, Predicate>,
        frame: &Frame,
        now_ms: UnixMs,
        assets: &BTreeMap<String, Vec<u8>>,
        program: Option<ProgramId>,
    ) -> Result<PredicateResult> {
        let pred = predicates
            .get(name)
            .ok_or_else(|| PerceptionError::UnknownPredicate(name.into()))?;
        let ctx = EvalCtx {
            frame,
            now_ms,
            cache: &self.cache,
            state: &self.state,
            predicates,
            assets,
            program,
        };
        let hash = hash_predicate(pred)?;
        if let Some(cached) = self.cache.get(&hash) {
            return Ok(cached);
        }
        let result = dispatch(pred, &ctx)?;
        self.cache.insert(hash, result.clone());
        Ok(result)
    }
}

impl Default for Perception {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash any predicate via the canonical JSON pipeline in `vcli-core`.
pub(crate) fn hash_predicate(p: &Predicate) -> Result<PredicateHash> {
    let v = serde_json::to_value(p)
        .map_err(|e| PerceptionError::AssetDecode(format!("serialize predicate for hash: {e}")))?;
    predicate_hash(&v).map_err(|e| PerceptionError::AssetDecode(format!("hash predicate: {e}")))
}

/// Dispatch a predicate to its evaluator. Internal — logical evaluators
/// call `dispatch_leaf` for non-logical kinds while they handle their
/// own recursion.
pub(crate) fn dispatch(p: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
    match p {
        PredicateKind::ColorAt { .. } => ColorAtEvaluator.evaluate(p, ctx),
        PredicateKind::PixelDiff { .. } => PixelDiffEvaluator.evaluate(p, ctx),
        PredicateKind::Template { .. } => TemplateEvaluator.evaluate(p, ctx),
        PredicateKind::AllOf { .. } => AllOfEvaluator.evaluate(p, ctx),
        PredicateKind::AnyOf { .. } => AnyOfEvaluator.evaluate(p, ctx),
        PredicateKind::Not { .. } => NotEvaluator.evaluate(p, ctx),
        PredicateKind::ElapsedMsSinceTrue { .. } => ElapsedMsSinceTrueEvaluator.evaluate(p, ctx),
    }
}

/// Non-logical dispatch. Called by `logical.rs` during recursion.
pub(crate) fn dispatch_leaf(p: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
    match p {
        PredicateKind::ColorAt { .. } => ColorAtEvaluator.evaluate(p, ctx),
        PredicateKind::PixelDiff { .. } => PixelDiffEvaluator.evaluate(p, ctx),
        PredicateKind::Template { .. } => TemplateEvaluator.evaluate(p, ctx),
        PredicateKind::ElapsedMsSinceTrue { .. } => ElapsedMsSinceTrueEvaluator.evaluate(p, ctx),
        PredicateKind::AllOf { .. } | PredicateKind::AnyOf { .. } | PredicateKind::Not { .. } => {
            // Logical kinds shouldn't flow through dispatch_leaf — the
            // logical evaluators handle their own sub-children. Defensive
            // fallback so we don't silently return the wrong thing.
            dispatch(p, ctx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::Rgb;
    use vcli_core::{Frame, FrameFormat};

    fn red_frame() -> Frame {
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            4,
            Arc::from(vec![255u8, 0, 0, 255]),
            0,
        )
    }

    #[test]
    fn evaluate_named_returns_truthy_for_matching_color() {
        let p = Perception::new();
        let mut preds = BTreeMap::new();
        preds.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let frame = red_frame();
        let r = p
            .evaluate_named("red_here", &preds, &frame, 123, &assets, None)
            .unwrap();
        assert!(r.truthy);
        assert_eq!(r.at, 123);
    }

    #[test]
    fn evaluate_named_unknown_errors() {
        let p = Perception::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let frame = red_frame();
        let r = p.evaluate_named("nope", &preds, &frame, 0, &assets, None);
        assert!(matches!(r, Err(PerceptionError::UnknownPredicate(_))));
    }

    #[test]
    fn second_call_for_same_name_is_a_cache_hit() {
        let p = Perception::new();
        let mut preds = BTreeMap::new();
        preds.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let frame = red_frame();
        let r1 = p
            .evaluate_named("red_here", &preds, &frame, 100, &assets, None)
            .unwrap();
        let r2 = p
            .evaluate_named("red_here", &preds, &frame, 200, &assets, None)
            .unwrap();
        // Same hash → same cached result → `at` timestamp preserved from
        // first eval, not updated to 200. This proves the cache hit.
        assert_eq!(r1.at, 100);
        assert_eq!(r2.at, 100);
    }

    #[test]
    fn clear_wipes_cache_but_not_state() {
        let p = Perception::new();
        let mut preds = BTreeMap::new();
        preds.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let frame = red_frame();
        let _ = p
            .evaluate_named("red_here", &preds, &frame, 100, &assets, None)
            .unwrap();
        assert_eq!(p.cache().len(), 1);
        p.clear();
        assert_eq!(p.cache().len(), 0);
        // State is preserved — recording a snapshot then clearing cache
        // keeps the snapshot accessible.
        let h = super::hash_predicate(&PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([255, 0, 0]),
            tolerance: 0,
        })
        .unwrap();
        p.state().record_snapshot(
            h.clone(),
            crate::state::PriorSnapshot {
                dhash: 42,
                at_ms: 1000,
            },
        );
        p.clear();
        assert!(p.state().prior_snapshot(&h).is_some());
    }

    #[test]
    fn two_programs_with_same_predicate_share_cache() {
        // If two programs both reference a predicate with the same canonical
        // JSON form (same kind, same params), the second evaluation should
        // hit the cache.
        let p = Perception::new();
        let mut preds = BTreeMap::new();
        preds.insert(
            "a".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        preds.insert(
            "b_also_red".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let frame = red_frame();
        let _ = p
            .evaluate_named("a", &preds, &frame, 100, &assets, None)
            .unwrap();
        let _ = p
            .evaluate_named("b_also_red", &preds, &frame, 200, &assets, None)
            .unwrap();
        // Only one entry — both names hashed to the same PredicateHash.
        assert_eq!(p.cache().len(), 1);
    }
}
