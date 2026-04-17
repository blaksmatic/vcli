//! Predicate definitions and evaluation results.
//!
//! Each `Predicate` kind maps to a `PredicateEvaluator` impl (defined in
//! `vcli-perception`). `PredicateResult` is what evaluators return and what
//! the cache stores.

use serde::{Deserialize, Serialize};

use crate::clock::UnixMs;
use crate::geom::Rect;
use crate::region::Region;

/// Confidence as a `f32` in [0, 1]. Newtyped to avoid silent mismatches with
/// arbitrary f32s and to prevent accidental equality checks that surprise.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Confidence(pub f32);

/// RGB triple for color matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rgb(pub [u8; 3]);

/// Predicate kinds supported in v0.
///
/// Post-v0 kinds (`ocr`, `ocr_text`, `vlm`) plug in via the same evaluator
/// trait without DSL schema churn — they'll appear as additional variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PredicateKind {
    /// Template match against a named image asset within a region.
    Template {
        /// Either a file path resolved at submit, or `sha256:<hex>` once
        /// rewritten by the daemon's submit module (Decision F4).
        image: String,
        /// Required confidence for truthy. Inclusive.
        confidence: Confidence,
        /// Where on screen to search.
        region: Region,
        /// Minimum milliseconds between evaluations (Tier 2).
        #[serde(default = "default_template_throttle_ms")]
        throttle_ms: u32,
    },
    /// Single-pixel color match.
    ColorAt {
        /// Logical-pixel coordinate.
        point: crate::geom::Point,
        /// Expected RGB.
        rgb: Rgb,
        /// Max Euclidean RGB distance considered a match.
        tolerance: u16,
    },
    /// Perceptual-hash comparison of a region against a baseline.
    PixelDiff {
        /// Region to hash.
        region: Region,
        /// `sha256:<hex>` baseline asset reference (stored as the image's
        /// content hash; the evaluator rehashes the region into a perceptual
        /// hash at eval time and compares).
        baseline: String,
        /// Fractional Hamming-distance threshold (0.0..=1.0).
        threshold: f32,
    },
    /// Logical AND across named predicates.
    AllOf {
        /// Names of predicates in the same program.
        of: Vec<String>,
    },
    /// Logical OR across named predicates.
    AnyOf {
        /// Names of predicates in the same program.
        of: Vec<String>,
    },
    /// Logical negation.
    Not {
        /// Predicate name in the same program.
        of: String,
    },
    /// True when the referenced predicate has been continuously truthy for at
    /// least `ms` milliseconds. Per-program state.
    ElapsedMsSinceTrue {
        /// Predicate name in the same program.
        predicate: String,
        /// Threshold in milliseconds.
        ms: u32,
    },
}

fn default_template_throttle_ms() -> u32 {
    200
}

/// Full predicate definition: the kind plus optional cross-program controls
/// carried in future extensions (v0 has none).
pub type Predicate = PredicateKind;

/// What an evaluator returns and what the `PerceptionCache` stores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredicateResult {
    /// Whether the predicate's truth condition held at eval time.
    pub truthy: bool,
    /// Location/confidence data, if the kind produces one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_data: Option<MatchData>,
    /// Unix ms when this result was computed.
    pub at: UnixMs,
}

/// Match metadata for location-producing predicate kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchData {
    /// Match bounding box in logical pixels (absolute screen coords).
    #[serde(rename = "box")]
    pub bbox: Rect,
    /// Reported confidence from the evaluator.
    pub confidence: Confidence,
}

impl MatchData {
    /// Convenience — bounding box center (matches `$p.match.center` in DSL
    /// expressions).
    #[must_use]
    pub fn center(&self) -> crate::geom::Point {
        self.bbox.center()
    }

    /// Convenience — top-left corner.
    #[must_use]
    pub fn top_left(&self) -> crate::geom::Point {
        self.bbox.top_left()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Point, Rect};
    use crate::region::Region;

    #[test]
    fn template_kind_roundtrip_with_default_throttle() {
        let j = r#"{"kind":"template","image":"assets/x.png","confidence":0.9,
                    "region":{"kind":"absolute","box":{"x":0,"y":0,"w":10,"h":10}}}"#;
        let p: PredicateKind = serde_json::from_str(j).unwrap();
        match &p {
            PredicateKind::Template { throttle_ms, .. } => assert_eq!(*throttle_ms, 200),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn color_at_roundtrip() {
        let p = PredicateKind::ColorAt {
            point: Point { x: 10, y: 20 },
            rgb: Rgb([255, 0, 128]),
            tolerance: 15,
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: PredicateKind = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn pixel_diff_roundtrip() {
        let p = PredicateKind::PixelDiff {
            region: Region::Absolute { rect: Rect { x: 0, y: 0, w: 50, h: 50 } },
            baseline: "sha256:abcd".into(),
            threshold: 0.05,
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: PredicateKind = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn all_of_and_any_of_and_not_roundtrips() {
        for p in [
            PredicateKind::AllOf { of: vec!["a".into(), "b".into()] },
            PredicateKind::AnyOf { of: vec!["a".into()] },
            PredicateKind::Not { of: "a".into() },
        ] {
            let j = serde_json::to_string(&p).unwrap();
            let back: PredicateKind = serde_json::from_str(&j).unwrap();
            assert_eq!(back, p);
        }
    }

    #[test]
    fn elapsed_ms_since_true_roundtrip() {
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "visible".into(),
            ms: 500,
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: PredicateKind = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn match_data_center_and_top_left() {
        let m = MatchData {
            bbox: Rect { x: 10, y: 10, w: 40, h: 20 },
            confidence: Confidence(0.95),
        };
        assert_eq!(m.center(), Point { x: 30, y: 20 });
        assert_eq!(m.top_left(), Point { x: 10, y: 10 });
    }

    #[test]
    fn predicate_result_no_match_serializes_without_match_field() {
        let r = PredicateResult { truthy: false, match_data: None, at: 1 };
        let j = serde_json::to_string(&r).unwrap();
        assert!(!j.contains("match"), "got {j}");
    }
}
