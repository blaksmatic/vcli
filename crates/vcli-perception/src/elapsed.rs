//! `ElapsedMsSinceTrueEvaluator` — Tier 1. True iff the referenced child
//! predicate has been continuously truthy for at least `ms` milliseconds.
//!
//! State is program-local (spec §"`elapsed_ms_since_true`"): timestamps
//! live in `PerceptionState::first_true_at` keyed by
//! `(program_id, predicate_name)`, not by predicate hash. Without a
//! program id in `EvalCtx`, this evaluator returns an error (the runtime
//! should never call it in that configuration).

use vcli_core::predicate::PredicateKind;
use vcli_core::{Predicate, PredicateResult};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::logical::crate_evaluate_named_with_depth;
use crate::state::ProgramPredKey;

/// Evaluator for `PredicateKind::ElapsedMsSinceTrue`.
#[derive(Debug, Default)]
pub struct ElapsedMsSinceTrueEvaluator;

impl Evaluator for ElapsedMsSinceTrueEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::ElapsedMsSinceTrue {
            predicate: child_name,
            ms,
        } = predicate
        else {
            return Err(PerceptionError::AssetDecode(
                "elapsed evaluator got wrong kind".into(),
            ));
        };

        let program = ctx.program.ok_or_else(|| {
            PerceptionError::AssetDecode(
                "elapsed_ms_since_true evaluated outside program context".into(),
            )
        })?;

        let child = crate_evaluate_named_with_depth(child_name, ctx, 0)?;

        let key = ProgramPredKey {
            program,
            predicate: child_name.clone(),
        };

        if child.truthy {
            // Rising edge: record the first-true timestamp. No-op if already set.
            ctx.state.set_first_true_at(key.clone(), ctx.now_ms);
            // Now check whether we've been true long enough.
            let first = ctx.state.first_true_at(&key).unwrap_or(ctx.now_ms);
            let elapsed = ctx.now_ms.saturating_sub(first);
            let truthy = elapsed >= i64::from(*ms);
            Ok(PredicateResult {
                truthy,
                match_data: None,
                at: ctx.now_ms,
            })
        } else {
            // Falling edge: clear so the next true starts a new run.
            ctx.state.clear_first_true_at(&key);
            Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::Rgb;
    use vcli_core::{Frame, FrameFormat, ProgramId};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    fn red_pixel_frame() -> Frame {
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

    fn make_preds(child_truthy_color: Rgb) -> BTreeMap<String, Predicate> {
        let mut m = BTreeMap::new();
        m.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: child_truthy_color,
                tolerance: 0,
            },
        );
        m
    }

    #[test]
    fn first_tick_truthy_child_not_elapsed_yet() {
        let state = PerceptionState::new();
        let cache = PredicateCache::new();
        let preds = make_preds(Rgb([255, 0, 0]));
        let assets = BTreeMap::new();
        let frame = red_pixel_frame();
        let pid = ProgramId::new();
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "red_here".into(),
            ms: 500,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: Some(pid),
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy, "0ms elapsed is not ≥500");
    }

    #[test]
    fn after_threshold_elapses_returns_truthy() {
        let state = PerceptionState::new();
        let cache = PredicateCache::new();
        let preds = make_preds(Rgb([255, 0, 0]));
        let assets = BTreeMap::new();
        let frame = red_pixel_frame();
        let pid = ProgramId::new();
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "red_here".into(),
            ms: 500,
        };
        let ctx1 = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: Some(pid),
        };
        let _ = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx1).unwrap();
        cache.clear();
        let ctx2 = EvalCtx {
            frame: &frame,
            now_ms: 1500, // 500ms later, still truthy
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: Some(pid),
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx2).unwrap();
        assert!(r.truthy);
    }

    #[test]
    fn falling_edge_clears_timer() {
        let state = PerceptionState::new();
        let cache = PredicateCache::new();
        let preds_red = make_preds(Rgb([255, 0, 0]));
        let preds_green = make_preds(Rgb([0, 255, 0]));
        let assets = BTreeMap::new();
        let frame = red_pixel_frame();
        let pid = ProgramId::new();
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "red_here".into(),
            ms: 500,
        };

        // Tick 1: child truthy, 0ms elapsed.
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds_red,
            assets: &assets,
            program: Some(pid),
        };
        let _ = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx).unwrap();

        // Tick 2: child becomes FALSE (expect green, frame is red).
        cache.clear();
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1200,
            cache: &cache,
            state: &state,
            predicates: &preds_green,
            assets: &assets,
            program: Some(pid),
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy);

        // Tick 3: back to truthy at t=2000. Should re-start the timer —
        // elapsed is 0, not 1000.
        cache.clear();
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 2000,
            cache: &cache,
            state: &state,
            predicates: &preds_red,
            assets: &assets,
            program: Some(pid),
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy, "timer restarted after falling edge");
    }

    #[test]
    fn no_program_id_errors() {
        let state = PerceptionState::new();
        let cache = PredicateCache::new();
        let preds = make_preds(Rgb([255, 0, 0]));
        let assets = BTreeMap::new();
        let frame = red_pixel_frame();
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "red_here".into(),
            ms: 500,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx);
        assert!(matches!(r, Err(PerceptionError::AssetDecode(_))));
    }
}
