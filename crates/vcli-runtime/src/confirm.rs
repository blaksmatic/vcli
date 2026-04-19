//! Input postcondition tracking.

use std::collections::BTreeMap;

use vcli_core::{Frame, Predicate, ProgramId, UnixMs};
use vcli_perception::Perception;

use crate::error::RuntimeError;

/// Pending postcondition: the scheduler re-evaluates this on subsequent ticks
/// until it flips truthy (success) or `deadline_ms` is reached (novelty timeout).
#[derive(Debug, Clone)]
pub struct PendingConfirm {
    /// Source program.
    pub program_id: ProgramId,
    /// Predicate name to watch.
    pub predicate: String,
    /// Timeout cap.
    pub deadline_ms: UnixMs,
    /// Step-path hint for `program.failed.step` if we time out.
    pub step_hint: String,
}

/// Outcome of one confirm step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// Postcondition became truthy — success.
    Success,
    /// Deadline elapsed — `novelty_timeout`.
    Timeout,
    /// Keep checking next tick.
    Pending,
}

/// Evaluate one pending confirm against the current tick.
///
/// # Errors
///
/// Propagates perception errors.
pub fn step(
    pc: &PendingConfirm,
    predicates: &BTreeMap<String, Predicate>,
    frame: &Frame,
    now_ms: UnixMs,
    assets: &BTreeMap<String, Vec<u8>>,
    perception: &Perception,
) -> Result<ConfirmOutcome, RuntimeError> {
    let r = perception
        .evaluate_named(
            &pc.predicate,
            predicates,
            frame,
            now_ms,
            assets,
            Some(pc.program_id),
        )
        .map_err(|e| RuntimeError::Perception(e.to_string()))?;
    if r.truthy {
        return Ok(ConfirmOutcome::Success);
    }
    if now_ms >= pc.deadline_ms {
        return Ok(ConfirmOutcome::Timeout);
    }
    Ok(ConfirmOutcome::Pending)
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

    fn id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn success_when_truthy() {
        let mut preds = BTreeMap::new();
        preds.insert(
            "red".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let pc = PendingConfirm {
            program_id: id(),
            predicate: "red".into(),
            deadline_ms: 1_000,
            step_hint: "body[0]".into(),
        };
        let p = Perception::new();
        let out = step(&pc, &preds, &red_frame(), 500, &BTreeMap::new(), &p).unwrap();
        assert_eq!(out, ConfirmOutcome::Success);
    }

    #[test]
    fn timeout_when_deadline_passed_and_still_false() {
        let mut preds = BTreeMap::new();
        preds.insert(
            "blue".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([0, 0, 255]),
                tolerance: 0,
            },
        );
        let pc = PendingConfirm {
            program_id: id(),
            predicate: "blue".into(),
            deadline_ms: 500,
            step_hint: "body[0]".into(),
        };
        let p = Perception::new();
        let out = step(&pc, &preds, &red_frame(), 800, &BTreeMap::new(), &p).unwrap();
        assert_eq!(out, ConfirmOutcome::Timeout);
    }

    #[test]
    fn pending_when_still_false_within_budget() {
        let mut preds = BTreeMap::new();
        preds.insert(
            "blue".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([0, 0, 255]),
                tolerance: 0,
            },
        );
        let pc = PendingConfirm {
            program_id: id(),
            predicate: "blue".into(),
            deadline_ms: 1_000,
            step_hint: "body[0]".into(),
        };
        let p = Perception::new();
        let out = step(&pc, &preds, &red_frame(), 300, &BTreeMap::new(), &p).unwrap();
        assert_eq!(out, ConfirmOutcome::Pending);
    }
}
