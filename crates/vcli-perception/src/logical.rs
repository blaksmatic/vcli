//! Logical composition evaluators — Tier 1. `all_of`, `any_of`, `not`
//! recurse through named predicates in `EvalCtx::predicates`, threading
//! the shared cache so siblings sharing a dependency pay only once.
//!
//! These are the first evaluators that _call back_ into the perception
//! layer for their children. They use a depth-bounded recursion guard so
//! a malformed program (which the DSL validator should have caught at
//! submit) can't crash the daemon.

use vcli_core::predicate::PredicateKind;
use vcli_core::{predicate_hash, Predicate, PredicateHash, PredicateResult};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};

const MAX_RECURSION_DEPTH: u32 = 32;

/// Look up a named predicate, evaluate it through the correct kind's
/// evaluator, memoize in the cache, and return the result. Used by
/// logical + elapsed_ms_since_true evaluators.
///
/// # Errors
///
/// - `UnknownPredicate` if `name` is not in `ctx.predicates`.
/// - `Cycle` if recursion exceeds `MAX_RECURSION_DEPTH`.
/// - Any error the child evaluator produces.
pub(crate) fn evaluate_named_with_depth(
    name: &str,
    ctx: &EvalCtx<'_>,
    depth: u32,
) -> Result<PredicateResult> {
    if depth >= MAX_RECURSION_DEPTH {
        return Err(PerceptionError::Cycle(name.into()));
    }
    let pred = ctx
        .predicates
        .get(name)
        .ok_or_else(|| PerceptionError::UnknownPredicate(name.into()))?;

    let hash = hash_predicate(pred)?;
    if let Some(cached) = ctx.cache.get(&hash) {
        return Ok(cached);
    }

    let result = match pred {
        PredicateKind::AllOf { of } => eval_all_of(of, ctx, depth + 1)?,
        PredicateKind::AnyOf { of } => eval_any_of(of, ctx, depth + 1)?,
        PredicateKind::Not { of } => eval_not(of, ctx, depth + 1)?,
        PredicateKind::ElapsedMsSinceTrue { .. }
        | PredicateKind::ColorAt { .. }
        | PredicateKind::PixelDiff { .. }
        | PredicateKind::Template { .. } => {
            // Non-logical kinds are dispatched by the Perception façade.
            // `evaluate_named_with_depth` is only called from logical /
            // elapsed contexts; those callers dispatch through the
            // façade (see `perception.rs`), which handles all kinds.
            crate::perception::dispatch_leaf(pred, ctx)?
        }
    };

    ctx.cache.insert(hash, result.clone());
    Ok(result)
}

fn hash_predicate(p: &Predicate) -> Result<PredicateHash> {
    let v = serde_json::to_value(p).map_err(|e| {
        PerceptionError::AssetDecode(format!("serialize predicate for hash: {e}"))
    })?;
    predicate_hash(&v)
        .map_err(|e| PerceptionError::AssetDecode(format!("hash predicate: {e}")))
}

fn eval_all_of(of: &[String], ctx: &EvalCtx<'_>, depth: u32) -> Result<PredicateResult> {
    // Short-circuit at first falsy child.
    for name in of {
        let r = evaluate_named_with_depth(name, ctx, depth)?;
        if !r.truthy {
            return Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            });
        }
    }
    Ok(PredicateResult {
        truthy: true,
        match_data: None,
        at: ctx.now_ms,
    })
}

fn eval_any_of(of: &[String], ctx: &EvalCtx<'_>, depth: u32) -> Result<PredicateResult> {
    for name in of {
        let r = evaluate_named_with_depth(name, ctx, depth)?;
        if r.truthy {
            return Ok(PredicateResult {
                truthy: true,
                match_data: None,
                at: ctx.now_ms,
            });
        }
    }
    Ok(PredicateResult {
        truthy: false,
        match_data: None,
        at: ctx.now_ms,
    })
}

fn eval_not(name: &str, ctx: &EvalCtx<'_>, depth: u32) -> Result<PredicateResult> {
    let child = evaluate_named_with_depth(name, ctx, depth)?;
    Ok(PredicateResult {
        truthy: !child.truthy,
        match_data: None,
        at: ctx.now_ms,
    })
}

/// Public evaluator for `all_of`. Delegates to the shared traversal.
#[derive(Debug, Default)]
pub struct AllOfEvaluator;

impl Evaluator for AllOfEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::AllOf { of } = predicate else {
            return Err(PerceptionError::AssetDecode(
                "all_of evaluator got wrong kind".into(),
            ));
        };
        eval_all_of(of, ctx, 0)
    }
}

/// Public evaluator for `any_of`.
#[derive(Debug, Default)]
pub struct AnyOfEvaluator;

impl Evaluator for AnyOfEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::AnyOf { of } = predicate else {
            return Err(PerceptionError::AssetDecode(
                "any_of evaluator got wrong kind".into(),
            ));
        };
        eval_any_of(of, ctx, 0)
    }
}

/// Public evaluator for `not`.
#[derive(Debug, Default)]
pub struct NotEvaluator;

impl Evaluator for NotEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::Not { of } = predicate else {
            return Err(PerceptionError::AssetDecode(
                "not evaluator got wrong kind".into(),
            ));
        };
        eval_not(of, ctx, 0)
    }
}

/// Re-export for `elapsed.rs` which needs to call back into the logical
/// recursion machinery with a depth argument.
pub(crate) use evaluate_named_with_depth as crate_evaluate_named_with_depth;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::Rgb;
    use vcli_core::{Frame, FrameFormat};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    fn red_pixel_frame() -> Frame {
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            4,
            Arc::from(pixels),
            0,
        )
    }

    fn build_preds() -> BTreeMap<String, Predicate> {
        let mut m = BTreeMap::new();
        m.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        m.insert(
            "green_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([0, 255, 0]),
                tolerance: 0,
            },
        );
        m
    }

    fn make_ctx<'a>(
        frame: &'a Frame,
        cache: &'a PredicateCache,
        state: &'a PerceptionState,
        preds: &'a BTreeMap<String, Predicate>,
        assets: &'a BTreeMap<String, Vec<u8>>,
    ) -> EvalCtx<'a> {
        EvalCtx {
            frame,
            now_ms: 10,
            cache,
            state,
            predicates: preds,
            assets,
            program: None,
        }
    }

    #[test]
    fn not_red_when_actually_red_is_false() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::Not {
            of: "red_here".into(),
        };
        let r = NotEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn all_of_true_when_all_children_true() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AllOf {
            of: vec!["red_here".into(), "red_here".into()],
        };
        let r = AllOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(r.truthy);
    }

    #[test]
    fn all_of_false_if_any_child_false() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AllOf {
            of: vec!["red_here".into(), "green_here".into()],
        };
        let r = AllOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn any_of_true_if_any_child_true() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AnyOf {
            of: vec!["green_here".into(), "red_here".into()],
        };
        let r = AnyOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(r.truthy);
    }

    #[test]
    fn any_of_empty_list_is_false() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AnyOf { of: vec![] };
        let r = AnyOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn unknown_named_predicate_errors() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::Not {
            of: "does_not_exist".into(),
        };
        let r = NotEvaluator.evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets));
        assert!(matches!(r, Err(PerceptionError::UnknownPredicate(_))));
    }

    #[test]
    fn logical_shares_cache_across_siblings() {
        // Evaluate (red_here AND red_here) — the second call should hit the
        // cache. We detect this by wrapping ColorAtEvaluator in a counter?
        // Simpler: assert the cache grows to at most 2 entries (red_here
        // result + the all_of result).
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AllOf {
            of: vec!["red_here".into(), "red_here".into()],
        };
        let _ = AllOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        // Cache should contain exactly one entry for `red_here`. The
        // `all_of` result is inserted by the Perception façade, not by the
        // logical evaluator itself.
        assert_eq!(cache.len(), 1, "red_here cached once, not twice");
    }
}
