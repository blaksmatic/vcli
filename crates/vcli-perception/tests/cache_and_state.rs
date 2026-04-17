//! Integration tests that exercise Perception as a whole:
//!
//! - Multi-program cache dedup for identical predicate canonical forms
//! - Per-tick cache wipe via `clear()`
//! - Cross-tick PerceptionState preservation for elapsed_ms_since_true

use std::collections::BTreeMap;
use std::sync::Arc;

use vcli_core::geom::{Point, Rect};
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::{Frame, FrameFormat, Predicate, ProgramId};

use vcli_perception::Perception;

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

fn color_pred(r: u8, g: u8, b: u8) -> Predicate {
    PredicateKind::ColorAt {
        point: Point { x: 0, y: 0 },
        rgb: Rgb([r, g, b]),
        tolerance: 0,
    }
}

#[test]
fn cache_dedups_across_distinct_program_predicate_names() {
    let p = Perception::new();
    let mut preds = BTreeMap::new();
    // Two programs, each with their own local name, but identical canonical form.
    preds.insert("prog_a_red".into(), color_pred(255, 0, 0));
    preds.insert("prog_b_red".into(), color_pred(255, 0, 0));
    let assets = BTreeMap::new();
    let frame = red_frame();
    let _ = p
        .evaluate_named("prog_a_red", &preds, &frame, 100, &assets, None)
        .unwrap();
    let _ = p
        .evaluate_named("prog_b_red", &preds, &frame, 200, &assets, None)
        .unwrap();
    assert_eq!(p.cache().len(), 1, "identical canonical predicates dedupe");
}

#[test]
fn per_tick_clear_invalidates_results() {
    let p = Perception::new();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), color_pred(255, 0, 0));
    let assets = BTreeMap::new();
    let frame = red_frame();
    let r1 = p
        .evaluate_named("red", &preds, &frame, 100, &assets, None)
        .unwrap();
    p.clear();
    let r2 = p
        .evaluate_named("red", &preds, &frame, 200, &assets, None)
        .unwrap();
    // Without cache clear, r2.at would equal r1.at (100). With it, r2 is
    // freshly evaluated at 200.
    assert_eq!(r1.at, 100);
    assert_eq!(r2.at, 200);
}

#[test]
fn elapsed_ms_since_true_persists_across_tick_clears() {
    let p = Perception::new();
    let pid = ProgramId::new();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), color_pred(255, 0, 0));
    preds.insert(
        "stable_500".into(),
        PredicateKind::ElapsedMsSinceTrue {
            predicate: "red".into(),
            ms: 500,
        },
    );
    let assets = BTreeMap::new();
    let frame = red_frame();

    // Tick 1 at t=1000: child true, 0ms elapsed → stable_500 is false.
    let r = p
        .evaluate_named("stable_500", &preds, &frame, 1000, &assets, Some(pid))
        .unwrap();
    assert!(!r.truthy);

    // End of tick — runtime clears the cache.
    p.clear();

    // Tick 2 at t=1499: 499ms elapsed, still false.
    let r = p
        .evaluate_named("stable_500", &preds, &frame, 1499, &assets, Some(pid))
        .unwrap();
    assert!(!r.truthy);

    p.clear();

    // Tick 3 at t=1500: exactly 500ms — truthy.
    let r = p
        .evaluate_named("stable_500", &preds, &frame, 1500, &assets, Some(pid))
        .unwrap();
    assert!(r.truthy);
}
