//! (stub — filled in by Task 13; dispatch_leaf is pre-wired so logical
//! evaluator tests pass in Task 10)
#![allow(missing_docs)]

use vcli_core::predicate::PredicateKind;
use vcli_core::{Predicate, PredicateResult};

use crate::color_at::ColorAtEvaluator;
use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::pixel_diff::PixelDiffEvaluator;

pub(crate) fn dispatch_leaf(p: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
    match p {
        PredicateKind::ColorAt { .. } => ColorAtEvaluator.evaluate(p, ctx),
        PredicateKind::PixelDiff { .. } => PixelDiffEvaluator.evaluate(p, ctx),
        // Template and ElapsedMsSinceTrue not available in the stub;
        // logical tests only use ColorAt/PixelDiff children.
        _ => Err(PerceptionError::AssetDecode(
            "dispatch_leaf stub: kind not yet wired".into(),
        )),
    }
}
