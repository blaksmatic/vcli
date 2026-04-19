//! Trigger evaluation. A program in `Waiting` advances to `Running` as soon
//! as its trigger fires. Spec §425.

use std::collections::BTreeMap;

use vcli_core::{Frame, Predicate, ProgramId, Trigger, UnixMs};
use vcli_perception::Perception;

use crate::error::RuntimeError;

/// Return `true` if the trigger says the program may start this tick.
///
/// `Manual` triggers never auto-fire; they require a `SchedulerCommand::Start`.
///
/// # Errors
///
/// Propagates [`RuntimeError::Perception`] when evaluating `OnPredicate`.
pub fn trigger_fires(
    trig: &Trigger,
    predicates: &BTreeMap<String, Predicate>,
    frame: &Frame,
    now_ms: UnixMs,
    assets: &BTreeMap<String, Vec<u8>>,
    perception: &Perception,
    program_id: ProgramId,
) -> Result<bool, RuntimeError> {
    match trig {
        Trigger::OnSubmit => Ok(true),
        Trigger::Manual => Ok(false),
        Trigger::OnPredicate { name } => {
            let r = perception
                .evaluate_named(name, predicates, frame, now_ms, assets, Some(program_id))
                .map_err(|e| RuntimeError::Perception(e.to_string()))?;
            Ok(r.truthy)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vcli_core::frame::FrameFormat;
    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::{PredicateKind, Rgb};

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

    fn some_id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn on_submit_always_fires() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let out = trigger_fires(
            &Trigger::OnSubmit,
            &preds,
            &red_frame(),
            0,
            &assets,
            &p,
            some_id(),
        )
        .unwrap();
        assert!(out);
    }

    #[test]
    fn manual_never_auto_fires() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let out = trigger_fires(
            &Trigger::Manual,
            &preds,
            &red_frame(),
            0,
            &assets,
            &p,
            some_id(),
        )
        .unwrap();
        assert!(!out);
    }

    #[test]
    fn on_predicate_defers_to_perception() {
        let mut preds = BTreeMap::new();
        preds.insert(
            "is_red".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let p = Perception::new();
        let out = trigger_fires(
            &Trigger::OnPredicate {
                name: "is_red".into(),
            },
            &preds,
            &red_frame(),
            100,
            &assets,
            &p,
            some_id(),
        )
        .unwrap();
        assert!(out);
    }

    #[test]
    fn on_predicate_unknown_name_errors() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let out = trigger_fires(
            &Trigger::OnPredicate {
                name: "nope".into(),
            },
            &preds,
            &red_frame(),
            0,
            &assets,
            &p,
            some_id(),
        );
        assert!(matches!(out, Err(RuntimeError::Perception(_))));
    }
}
