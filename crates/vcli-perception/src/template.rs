//! `TemplateEvaluator` — Tier 2. NCC template matching via
//! `imageproc::template_matching::match_template` with
//! `MatchTemplateMethod::CrossCorrelationNormalized`.
//!
//! Spec §"Template matching": `imageproc` NCC, region-scoped by default
//! (the runtime passes the region as `Region::Absolute` after resolving
//! `window` / `relative_to`). Confidence is an inclusive threshold in
//! [0, 1]. v0 **does not** implement pyramid search — Decision 4.2
//! defers that to the runtime/submit pipeline (it only affects how
//! templates are stored, not the evaluator's interface).

use image::{imageops, GrayImage};
use imageproc::template_matching::{find_extremes, match_template, MatchTemplateMethod};

use vcli_core::geom::Rect;
use vcli_core::predicate::{Confidence, PredicateKind};
use vcli_core::{MatchData, Predicate, PredicateResult, Region};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::frame_view::{crop_rgb, frame_to_rgb};

/// Evaluator for `PredicateKind::Template`.
#[derive(Debug, Default)]
pub struct TemplateEvaluator;

impl TemplateEvaluator {
    /// Decode a PNG byte-slice into a grayscale image. Used by the
    /// evaluator to load the asset the daemon materialized into
    /// `EvalCtx::assets`.
    ///
    /// # Errors
    ///
    /// Returns `AssetDecode` if `image::load_from_memory` fails.
    pub fn decode_gray(bytes: &[u8]) -> Result<GrayImage> {
        let dyn_img = image::load_from_memory(bytes)
            .map_err(|e| PerceptionError::AssetDecode(e.to_string()))?;
        Ok(dyn_img.to_luma8())
    }
}

impl Evaluator for TemplateEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::Template {
            image: image_ref,
            confidence,
            region,
            throttle_ms: _,
        } = predicate
        else {
            return Err(PerceptionError::AssetDecode(
                "template evaluator got wrong kind".into(),
            ));
        };

        // Resolve the asset bytes. Daemon strips the "sha256:" prefix
        // before handing bytes to us, so the key is the hex digest.
        let key = image_ref.strip_prefix("sha256:").unwrap_or(image_ref);
        let bytes = ctx
            .assets
            .get(key)
            .ok_or_else(|| PerceptionError::AssetNotMaterialized(image_ref.clone()))?;
        let template_gray = Self::decode_gray(bytes)?;

        // Select the haystack: either the full frame or a cropped region.
        let (haystack_rgb, region_origin) = match region {
            Region::Absolute { rect } => {
                let crop = crop_rgb(ctx.frame, *rect)?;
                (
                    crop,
                    (
                        rect.x.max(ctx.frame.bounds.x),
                        rect.y.max(ctx.frame.bounds.y),
                    ),
                )
            }
            // Window / RelativeTo should have been resolved to Absolute
            // by the runtime before this evaluator is called. Treat as
            // the whole frame as a graceful fallback.
            _ => (
                frame_to_rgb(ctx.frame)?,
                (ctx.frame.bounds.x, ctx.frame.bounds.y),
            ),
        };
        let haystack_gray = imageops::grayscale(&haystack_rgb);

        // Haystack must be at least as big as the template.
        if haystack_gray.width() < template_gray.width()
            || haystack_gray.height() < template_gray.height()
        {
            return Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            });
        }

        // Normalized cross-correlation. Returns a response map where
        // higher values = better match; max is 1.0 for a perfect match.
        let result_map = match_template(
            &haystack_gray,
            &template_gray,
            MatchTemplateMethod::CrossCorrelationNormalized,
        );
        let extremes = find_extremes(&result_map);
        let (best_val, best_loc) = (extremes.max_value, extremes.max_value_location);

        // `best_val` is in [0, 1] for normalized cross-correlation.
        let best_val_f64 = f64::from(best_val);
        let threshold = confidence.0;

        if best_val_f64 < threshold {
            return Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            });
        }

        // Translate the match location from haystack-local to absolute
        // screen coordinates. best_loc and template dimensions are u32 from
        // imageproc; practical frame dimensions fit safely in i32.
        #[allow(clippy::cast_possible_wrap)]
        let bbox = Rect {
            x: region_origin.0 + best_loc.0 as i32,
            y: region_origin.1 + best_loc.1 as i32,
            w: template_gray.width() as i32,
            h: template_gray.height() as i32,
        };
        Ok(PredicateResult {
            truthy: true,
            match_data: Some(MatchData {
                bbox,
                confidence: Confidence(best_val_f64),
            }),
            at: ctx.now_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use image::{ImageBuffer, Luma, Rgb};

    use vcli_core::geom::Rect;
    use vcli_core::{Frame, FrameFormat, Region};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    /// Build a 64×64 frame that contains the `template_png_varied_16x8`
    /// pattern at position (20, 20): a white background with a white 16×8
    /// patch at (20,20) that has a black 8×4 inner block at (24, 22).
    /// This matches the template created by `template_png_varied_16x8`.
    fn scene_with_box() -> Frame {
        let w = 64usize;
        let h = 64usize;
        let stride = w * 4;
        // Start all white.
        let mut pixels = vec![255u8; stride * h];
        // Place the black inner block at absolute (24, 22) = (20+4, 20+2)
        for y in 22..26usize {
            for x in 24..32usize {
                let off = y * stride + x * 4;
                pixels[off] = 0;
                pixels[off + 1] = 0;
                pixels[off + 2] = 0;
                pixels[off + 3] = 255;
            }
        }
        // w=64, h=64 — safe to cast to i32.
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        let (wi, hi) = (w as i32, h as i32);
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: wi,
                h: hi,
            },
            stride,
            Arc::from(pixels),
            0,
        )
    }

    /// Build a PNG byte-stream for a 16×8 template that has variation:
    /// a black center (8×4) surrounded by a white border. This ensures
    /// non-zero variance so NCC gives a meaningful score.
    /// Note: NCC with a constant (all-black) template returns 0 due to
    /// zero variance — templates must have variation to work with NCC.
    fn template_png_varied_16x8() -> Vec<u8> {
        let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(16, 8, Rgb([255, 255, 255]));
        // Fill inner 8×4 region with black to give variation.
        for y in 2..6u32 {
            for x in 4..12u32 {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
        let mut bytes: Vec<u8> = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        bytes
    }

    fn template_png_white_16x8() -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(16, 8, Rgb([255, 255, 255]));
        let mut bytes: Vec<u8> = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        bytes
    }

    #[test]
    fn decode_gray_parses_valid_png() {
        let png = template_png_varied_16x8();
        let g = TemplateEvaluator::decode_gray(&png).unwrap();
        assert_eq!(g.width(), 16);
        assert_eq!(g.height(), 8);
    }

    #[test]
    fn decode_gray_errors_on_garbage() {
        let r = TemplateEvaluator::decode_gray(b"not a png");
        assert!(matches!(r, Err(PerceptionError::AssetDecode(_))));
    }

    #[test]
    fn template_match_found_in_scene_reports_correct_bbox() {
        // Note: NCC (CrossCorrelationNormalized) requires the template to have
        // non-zero variance. A constant (all-black) template yields NCC=0
        // everywhere due to zero variance. We use `template_png_varied_16x8`
        // (white border, black inner block) which has variation, giving NCC>0.
        let frame = scene_with_box();
        let template_bytes = template_png_varied_16x8();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let mut assets = BTreeMap::new();
        assets.insert("pattern".into(), template_bytes);
        let p = PredicateKind::Template {
            image: "sha256:pattern".into(),
            confidence: Confidence(0.5),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
            },
            throttle_ms: 200,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 42,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = TemplateEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(r.truthy, "should find the varied template in the scene");
        let m = r.match_data.expect("match_data populated on truthy");
        assert_eq!(m.bbox.w, 16);
        assert_eq!(m.bbox.h, 8);
        // Template placed so the black inner block lines up at (24,22)=origin+(4,2)
        assert_eq!(m.bbox.x, 20);
        assert_eq!(m.bbox.y, 20);
        assert!(m.confidence.0 >= 0.5);
    }

    #[test]
    fn template_missing_in_scene_is_falsy() {
        let frame = scene_with_box(); // black-box-on-white
        let template_bytes = template_png_white_16x8(); // all-white template never correlates high
                                                        // Actually: NCC of all-white against a region with all-white pixels
                                                        // can return 1.0 trivially, BUT find_extremes on constant images
                                                        // returns NaN due to zero variance — imageproc guards this by
                                                        // returning 0. Prefer using confidence 0.99 + a scene whose
                                                        // whitespace regions still won't exceed it. Simpler: use a
                                                        // distinctive template instead. Use a 32×32 gradient template
                                                        // that cannot be found in a 64x64 solid scene.
        let _ = template_bytes; // Unused: we build a different template below.

        let mut grad: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(32, 32);
        for y in 0..32u32 {
            for x in 0..32u32 {
                grad.put_pixel(
                    x,
                    y,
                    Rgb([((x * 8) % 255) as u8, ((y * 8) % 255) as u8, 128]),
                );
            }
        }
        let mut grad_bytes: Vec<u8> = Vec::new();
        grad.write_to(
            &mut std::io::Cursor::new(&mut grad_bytes),
            image::ImageFormat::Png,
        )
        .unwrap();

        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let mut assets = BTreeMap::new();
        assets.insert("grad".into(), grad_bytes);
        let p = PredicateKind::Template {
            image: "sha256:grad".into(),
            confidence: Confidence(0.99),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
            },
            throttle_ms: 200,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 42,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = TemplateEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy);
        assert!(r.match_data.is_none());
    }

    #[test]
    fn template_larger_than_haystack_is_falsy() {
        let frame = scene_with_box();
        // Build a template larger than the search region.
        let big: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_pixel(200, 200, Luma([0]));
        let mut bytes: Vec<u8> = Vec::new();
        big.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let mut assets = BTreeMap::new();
        assets.insert("big".into(), bytes);
        let p = PredicateKind::Template {
            image: "sha256:big".into(),
            confidence: Confidence(0.5),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
            },
            throttle_ms: 200,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 42,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = TemplateEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn missing_asset_errors() {
        let frame = scene_with_box();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new(); // no "blackbox" entry
        let p = PredicateKind::Template {
            image: "sha256:blackbox".into(),
            confidence: Confidence(0.9),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
            },
            throttle_ms: 200,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 42,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = TemplateEvaluator.evaluate(&p, &ctx);
        assert!(matches!(r, Err(PerceptionError::AssetNotMaterialized(_))));
    }
}
