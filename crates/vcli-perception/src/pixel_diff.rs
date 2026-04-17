//! `PixelDiffEvaluator` — Tier 1. Perceptual-hash (dHash) over a region,
//! compared to a prior-tick hash stored in `PerceptionState`.
//!
//! Spec §"Color / pixel diff": dHash over the region + Hamming distance
//! against the baseline. In v0 the "baseline" is the previous tick's
//! snapshot of the same region (this is what gives us a cheap motion
//! detector). When no prior snapshot exists, the predicate is falsy and
//! we just record the baseline for next tick.

use image::imageops::FilterType;
use image::{imageops, GrayImage};
use serde_json::json;

use vcli_core::predicate::PredicateKind;
use vcli_core::{canonicalize, Predicate, PredicateResult};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::frame_view::crop_rgb;
use crate::state::PriorSnapshot;

/// Stateless evaluator for `PredicateKind::PixelDiff`. State lives in
/// `PerceptionState::prior_snapshots`.
#[derive(Debug, Default)]
pub struct PixelDiffEvaluator;

impl PixelDiffEvaluator {
    /// Compute a 64-bit dHash over a grayscale thumbnail (9×8 resample).
    /// Bit `i` is set iff pixel `(x, y)` is brighter than `(x+1, y)` for
    /// the 8×8 block produced by taking the 9×8 differences.
    #[must_use]
    pub fn dhash(gray: &GrayImage) -> u64 {
        let resized = imageops::resize(gray, 9, 8, FilterType::Lanczos3);
        let mut h: u64 = 0;
        for y in 0..8u32 {
            for x in 0..8u32 {
                let l = resized.get_pixel(x, y).0[0];
                let r = resized.get_pixel(x + 1, y).0[0];
                h <<= 1;
                if l > r {
                    h |= 1;
                }
            }
        }
        h
    }

    /// Hamming distance between two 64-bit hashes.
    #[must_use]
    pub fn hamming(a: u64, b: u64) -> u32 {
        (a ^ b).count_ones()
    }

    /// Convenience: wrap an Rgb crop into a grayscale buffer.
    pub(crate) fn grayscale(img: &image::RgbImage) -> GrayImage {
        imageops::grayscale(img)
    }
}

impl Evaluator for PixelDiffEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::PixelDiff {
            region,
            baseline,
            threshold,
        } = predicate
        else {
            return Err(PerceptionError::AssetDecode(
                "pixel_diff evaluator received wrong predicate kind".into(),
            ));
        };

        // v0 scope: only Region::Absolute is handled directly here. Window
        // + RelativeTo are resolved by the runtime before calling us and
        // passed in as Absolute. If we see a non-absolute, treat as false
        // for v0 (the runtime should never hand us one).
        let rect = match region {
            vcli_core::Region::Absolute { rect } => *rect,
            _ => {
                return Ok(PredicateResult {
                    truthy: false,
                    match_data: None,
                    at: ctx.now_ms,
                });
            }
        };

        // Hash key for state lookup: predicate-local (baseline + region).
        let key_value = json!({
            "kind": "pixel_diff_state",
            "baseline": baseline,
            "rect": [rect.x, rect.y, rect.w, rect.h],
        });
        let key_bytes = canonicalize(&key_value).map_err(|e| {
            PerceptionError::AssetDecode(format!("canonicalize pixel_diff key: {e}"))
        })?;
        // SHA over canonical bytes via vcli_core::predicate_hash on the same value.
        let state_key = vcli_core::predicate_hash(&key_value).map_err(|e| {
            PerceptionError::AssetDecode(format!("hash pixel_diff key: {e}"))
        })?;
        let _ = key_bytes; // Retained for future diagnostics; canonicalize call asserts stability.

        // Compute current dHash of the region.
        let current_crop = crop_rgb(ctx.frame, rect)?;
        let current_gray = Self::grayscale(&current_crop);
        let current_hash = Self::dhash(&current_gray);

        let prior = ctx.state.prior_snapshot(&state_key);
        // Always record the current snapshot for the next tick.
        ctx.state.record_snapshot(
            state_key.clone(),
            PriorSnapshot {
                dhash: current_hash,
                at_ms: ctx.now_ms,
            },
        );

        // No prior snapshot → first tick, not truthy (nothing to diff).
        let Some(prev) = prior else {
            return Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            });
        };

        // Fraction-of-64 Hamming distance vs threshold.
        let distance = f64::from(Self::hamming(current_hash, prev.dhash)) / 64.0;
        let truthy = distance >= *threshold;
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

    use image::{Rgb, RgbImage};
    use vcli_core::geom::Rect;
    use vcli_core::predicate::PredicateKind;
    use vcli_core::{Frame, FrameFormat, Region};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    /// 32×32 solid RGBA frame filled with `color`.
    fn solid_frame(color: [u8; 3]) -> Frame {
        let w = 32usize;
        let h = 32usize;
        let stride = w * 4;
        let mut pixels = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let off = y * stride + x * 4;
                pixels[off] = color[0];
                pixels[off + 1] = color[1];
                pixels[off + 2] = color[2];
                pixels[off + 3] = 255;
            }
        }
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: w as i32,
                h: h as i32,
            },
            stride,
            Arc::from(pixels),
            0,
        )
    }

    #[test]
    fn dhash_and_hamming_on_identical_images_is_zero() {
        let img = RgbImage::from_pixel(32, 32, Rgb([128, 128, 128]));
        let g = PixelDiffEvaluator::grayscale(&img);
        let h1 = PixelDiffEvaluator::dhash(&g);
        let h2 = PixelDiffEvaluator::dhash(&g);
        assert_eq!(PixelDiffEvaluator::hamming(h1, h2), 0);
    }

    #[test]
    fn dhash_differs_for_contrasting_images() {
        let uniform = RgbImage::from_pixel(32, 32, Rgb([128, 128, 128]));
        let mut striped = RgbImage::from_pixel(32, 32, Rgb([0, 0, 0]));
        for y in 0..32u32 {
            for x in (0..32u32).step_by(2) {
                striped.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        let h1 = PixelDiffEvaluator::dhash(&PixelDiffEvaluator::grayscale(&uniform));
        let h2 = PixelDiffEvaluator::dhash(&PixelDiffEvaluator::grayscale(&striped));
        // Striping is dramatic — should flip most bits.
        assert!(PixelDiffEvaluator::hamming(h1, h2) > 16);
    }

    #[test]
    fn first_tick_not_truthy_but_records_snapshot() {
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let frame = solid_frame([200, 50, 50]);
        let p = PredicateKind::PixelDiff {
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 32,
                    h: 32,
                },
            },
            baseline: "sha256:unused".into(),
            threshold: 0.1,
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
        let r = PixelDiffEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn same_scene_on_second_tick_is_not_truthy() {
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let frame = solid_frame([200, 50, 50]);
        let p = PredicateKind::PixelDiff {
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 32,
                    h: 32,
                },
            },
            baseline: "sha256:unused".into(),
            threshold: 0.1,
        };
        // Tick 1: record snapshot.
        let ctx1 = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let _ = PixelDiffEvaluator.evaluate(&p, &ctx1).unwrap();
        // Tick 2: same frame, no diff.
        let ctx2 = EvalCtx {
            frame: &frame,
            now_ms: 1100,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = PixelDiffEvaluator.evaluate(&p, &ctx2).unwrap();
        assert!(!r.truthy, "identical frames should not diff");
    }

    #[test]
    fn changed_scene_on_second_tick_is_truthy() {
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();

        // Two frames that will produce very different dHashes.
        let frame_a = solid_frame([10, 10, 10]);
        // For B, alternate rows of black/white — drastically different gray.
        let mut frame_b_pixels = vec![0u8; 32 * 32 * 4];
        for y in 0..32usize {
            for x in 0..32usize {
                let off = y * 32 * 4 + x * 4;
                let v = if (x + y) % 2 == 0 { 0u8 } else { 255u8 };
                frame_b_pixels[off] = v;
                frame_b_pixels[off + 1] = v;
                frame_b_pixels[off + 2] = v;
                frame_b_pixels[off + 3] = 255;
            }
        }
        let frame_b = Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 32,
                h: 32,
            },
            32 * 4,
            Arc::from(frame_b_pixels),
            0,
        );

        let p = PredicateKind::PixelDiff {
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 32,
                    h: 32,
                },
            },
            baseline: "sha256:unused".into(),
            threshold: 0.1,
        };
        let ctx1 = EvalCtx {
            frame: &frame_a,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let _ = PixelDiffEvaluator.evaluate(&p, &ctx1).unwrap();
        let ctx2 = EvalCtx {
            frame: &frame_b,
            now_ms: 1100,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = PixelDiffEvaluator.evaluate(&p, &ctx2).unwrap();
        assert!(r.truthy);
    }
}
