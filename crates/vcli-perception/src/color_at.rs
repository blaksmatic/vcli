//! `ColorAtEvaluator` — Tier 1, <1ms. Samples one pixel and checks its
//! Euclidean RGB distance from the target.

use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::{Frame, Predicate, PredicateResult};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::frame_view::pixel_rgb;

/// Stateless evaluator for `PredicateKind::ColorAt`.
#[derive(Debug, Default)]
pub struct ColorAtEvaluator;

impl ColorAtEvaluator {
    /// Pure sampling helper. Public for tests; prefer `Evaluator::evaluate`.
    ///
    /// # Errors
    ///
    /// Propagates `RegionOutOfBounds` if the point is outside the frame.
    pub fn sample(frame: &Frame, x: i32, y: i32, target: Rgb, tolerance: u16) -> Result<bool> {
        let [r, g, b] = pixel_rgb(frame, x, y)?;
        let [tr, tg, tb] = target.0;
        let dr = i32::from(r) - i32::from(tr);
        let dg = i32::from(g) - i32::from(tg);
        let db = i32::from(b) - i32::from(tb);
        let dist_sq = dr * dr + dg * dg + db * db;
        let tol_sq = i32::from(tolerance) * i32::from(tolerance);
        Ok(dist_sq <= tol_sq)
    }
}

impl Evaluator for ColorAtEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::ColorAt {
            point,
            rgb,
            tolerance,
        } = predicate
        else {
            return Err(PerceptionError::AssetDecode(
                "color_at evaluator received non-color_at predicate".into(),
            ));
        };
        // Translate absolute screen point to frame-local.
        let fb = ctx.frame.bounds;
        let fx = point.x - fb.x;
        let fy = point.y - fb.y;
        let truthy = Self::sample(ctx.frame, fx, fy, *rgb, *tolerance)?;
        Ok(PredicateResult {
            truthy,
            match_data: None,
            at: ctx.now_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::Rgb;
    use vcli_core::{FrameFormat, PredicateKind};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    /// 2×2 RGBA8 frame: (0,0)=red, (1,0)=green, (0,1)=blue, (1,1)=white.
    fn rgba_checker() -> Frame {
        let pixels: Vec<u8> = vec![
            255, 0, 0, 255, // (0,0) red
            0, 255, 0, 255, // (1,0) green
            0, 0, 255, 255, // (0,1) blue
            255, 255, 255, 255, // (1,1) white
        ];
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            },
            8,
            Arc::from(pixels),
            0,
        )
    }

    fn ctx<'a>(
        frame: &'a Frame,
        cache: &'a PredicateCache,
        state: &'a PerceptionState,
        preds: &'a BTreeMap<String, Predicate>,
        assets: &'a BTreeMap<String, Vec<u8>>,
    ) -> EvalCtx<'a> {
        EvalCtx {
            frame,
            now_ms: 42,
            cache,
            state,
            predicates: preds,
            assets,
            program: None,
        }
    }

    #[test]
    fn exact_color_match_is_truthy() {
        let f = rgba_checker();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([255, 0, 0]),
            tolerance: 0,
        };
        let r = ColorAtEvaluator
            .evaluate(&p, &ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(r.truthy);
        assert_eq!(r.at, 42);
        assert!(r.match_data.is_none());
    }

    #[test]
    fn off_by_one_within_tolerance_is_truthy() {
        let f = rgba_checker();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = PredicateKind::ColorAt {
            point: Point { x: 1, y: 0 },
            rgb: Rgb([1, 254, 1]),
            tolerance: 5,
        };
        let r = ColorAtEvaluator
            .evaluate(&p, &ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(r.truthy);
    }

    #[test]
    fn beyond_tolerance_is_falsy() {
        let f = rgba_checker();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = PredicateKind::ColorAt {
            point: Point { x: 0, y: 1 }, // blue
            rgb: Rgb([255, 0, 0]),       // red target
            tolerance: 10,
        };
        let r = ColorAtEvaluator
            .evaluate(&p, &ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn out_of_bounds_errors() {
        let f = rgba_checker();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = PredicateKind::ColorAt {
            point: Point { x: 100, y: 100 },
            rgb: Rgb([0, 0, 0]),
            tolerance: 0,
        };
        let r = ColorAtEvaluator.evaluate(&p, &ctx(&f, &cache, &state, &preds, &assets));
        assert!(matches!(r, Err(PerceptionError::RegionOutOfBounds)));
    }
}
