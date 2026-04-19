//! Body step executor: runs exactly one step per tick; advances `body_cursor`
//! on success.

use std::collections::BTreeMap;
use std::sync::Arc;

use vcli_core::geom::Point;
use vcli_core::step::{OnFail, OnTimeout, Step, Target};
use vcli_core::{Frame, Predicate, ProgramId, UnixMs};
use vcli_input::InputSink;
use vcli_perception::Perception;

use crate::error::RuntimeError;
use crate::expr;

/// Outcome of one body-step attempt.
#[derive(Debug)]
pub enum StepOutcome {
    /// Step completed — advance `body_cursor` by 1.
    Advanced,
    /// Step is still waiting (e.g. `wait_for`, `sleep_ms` not yet elapsed). Do not advance.
    Stalled,
    /// Body finished (cursor already past end).
    BodyComplete,
    /// Program must transition to `failed`.
    Failed(RuntimeError),
}

/// Pending `sleep_ms` / `wait_for` deadline. The scheduler stashes this in
/// `RunningProgram` and the body executor consults it on subsequent ticks.
#[derive(Debug, Clone)]
pub enum BodyDefer {
    /// Sleep until `wake_at_ms`.
    Sleep {
        /// Wall-clock ms after which the sleep is done.
        wake_at_ms: UnixMs,
    },
    /// Wait for predicate up to `deadline_ms`.
    WaitFor {
        /// Predicate name.
        predicate: String,
        /// Deadline.
        deadline_ms: UnixMs,
        /// Timeout behaviour.
        on_timeout: OnTimeout,
    },
}

/// Per-program body-executor state.
#[derive(Default, Clone, Debug)]
pub struct BodyState {
    /// Active deferral.
    pub deferred: Option<BodyDefer>,
}

/// Execute one body step. The scheduler holds the borrow on `RunningProgram`,
/// so mutable fields (cursor + defer) live on `BodyState`; callers pass them
/// in.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
pub fn step_once(
    program_id: ProgramId,
    body: &[Step],
    cursor: u32,
    state: &mut BodyState,
    predicates: &BTreeMap<String, Predicate>,
    frame: &Frame,
    now_ms: UnixMs,
    assets: &BTreeMap<String, Vec<u8>>,
    perception: &Perception,
    input: &Arc<dyn InputSink>,
) -> StepOutcome {
    if let Some(def) = state.deferred.clone() {
        match def {
            BodyDefer::Sleep { wake_at_ms } => {
                if now_ms >= wake_at_ms {
                    state.deferred = None;
                    return StepOutcome::Advanced;
                }
                return StepOutcome::Stalled;
            }
            BodyDefer::WaitFor { predicate, deadline_ms, on_timeout } => {
                let r = match perception.evaluate_named(
                    &predicate, predicates, frame, now_ms, assets, Some(program_id),
                ) {
                    Ok(r) => r,
                    Err(e) => return StepOutcome::Failed(RuntimeError::Perception(e.to_string())),
                };
                if r.truthy {
                    state.deferred = None;
                    return StepOutcome::Advanced;
                }
                if now_ms >= deadline_ms {
                    state.deferred = None;
                    return match on_timeout {
                        OnTimeout::Continue | OnTimeout::Retry => StepOutcome::Advanced,
                        OnTimeout::Fail => StepOutcome::Failed(RuntimeError::WaitForTimeout {
                            predicate,
                            waited_ms: 0,
                        }),
                    };
                }
                return StepOutcome::Stalled;
            }
        }
    }

    let Some(step) = body.get(cursor as usize) else {
        return StepOutcome::BodyComplete;
    };

    match step {
        Step::SleepMs { ms } => {
            state.deferred = Some(BodyDefer::Sleep {
                wake_at_ms: now_ms.saturating_add(UnixMs::from(*ms)),
            });
            StepOutcome::Stalled
        }
        Step::WaitFor { predicate, timeout_ms, on_timeout } => {
            state.deferred = Some(BodyDefer::WaitFor {
                predicate: predicate.clone(),
                deadline_ms: now_ms.saturating_add(UnixMs::from(*timeout_ms)),
                on_timeout: *on_timeout,
            });
            StepOutcome::Stalled
        }
        Step::Assert { predicate, on_fail } => {
            let r = match perception.evaluate_named(
                predicate, predicates, frame, now_ms, assets, Some(program_id),
            ) {
                Ok(r) => r,
                Err(e) => return StepOutcome::Failed(RuntimeError::Perception(e.to_string())),
            };
            if r.truthy {
                StepOutcome::Advanced
            } else {
                match on_fail {
                    OnFail::Continue => StepOutcome::Advanced,
                    OnFail::Fail => StepOutcome::Failed(RuntimeError::AssertFailed {
                        predicate: predicate.clone(),
                    }),
                }
            }
        }
        Step::Move { at } => dispatch_at(at, predicates, frame, now_ms, assets, perception, program_id, |p| {
            input.mouse_move(p).map_err(|e| RuntimeError::Input(e.to_string()))
        }),
        Step::Click { at, button } => dispatch_at(at, predicates, frame, now_ms, assets, perception, program_id, |p| {
            input.click(p, *button, &[], 0).map_err(|e| RuntimeError::Input(e.to_string()))
        }),
        Step::Scroll { at, dx, dy } => {
            let _ = (dx, dy);
            dispatch_at(at, predicates, frame, now_ms, assets, perception, program_id, |p| {
                input.mouse_move(p).map_err(|e| RuntimeError::Input(e.to_string()))
            })
        }
        Step::Type { text } => match input.type_text(text) {
            Ok(()) => StepOutcome::Advanced,
            Err(e) => StepOutcome::Failed(RuntimeError::Input(e.to_string())),
        },
        Step::Key { key, modifiers } => match input.key_combo(modifiers, key) {
            Ok(()) => StepOutcome::Advanced,
            Err(e) => StepOutcome::Failed(RuntimeError::Input(e.to_string())),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_at<F>(
    target: &Target,
    predicates: &BTreeMap<String, Predicate>,
    frame: &Frame,
    now_ms: UnixMs,
    assets: &BTreeMap<String, Vec<u8>>,
    perception: &Perception,
    program_id: ProgramId,
    f: F,
) -> StepOutcome
where
    F: FnOnce(Point) -> Result<(), RuntimeError>,
{
    let point = match target {
        Target::Absolute(p) => *p,
        Target::Expression(s) => match expr::parse(s) {
            Ok(e) => {
                let r = match perception.evaluate_named(
                    e.predicate, predicates, frame, now_ms, assets, Some(program_id),
                ) {
                    Ok(r) => r,
                    Err(e2) => return StepOutcome::Failed(RuntimeError::Perception(e2.to_string())),
                };
                match e.accessor {
                    expr::Accessor::MatchCenter => match expr::resolve_center(&r) {
                        Ok(p) => p,
                        Err(e) => return StepOutcome::Failed(e),
                    },
                    expr::Accessor::MatchBbox => match expr::resolve_bbox(&r) {
                        Ok(bx) => Point { x: bx.x, y: bx.y },
                        Err(e) => return StepOutcome::Failed(e),
                    },
                }
            }
            Err(e) => return StepOutcome::Failed(e),
        },
    };
    match f(point) {
        Ok(()) => StepOutcome::Advanced,
        Err(e) => StepOutcome::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vcli_core::action::{Button, Modifier};
    use vcli_core::frame::FrameFormat;
    use vcli_core::geom::Rect;

    fn blank_frame() -> Frame {
        Frame::new(
            FrameFormat::Rgba8,
            Rect { x: 0, y: 0, w: 1, h: 1 },
            4,
            Arc::from(vec![0u8, 0, 0, 0]),
            0,
        )
    }

    fn id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    struct NopSink;
    impl InputSink for NopSink {
        fn mouse_move(&self, _: Point) -> Result<(), vcli_input::error::InputError> {
            Ok(())
        }
        fn click(
            &self,
            _: Point,
            _: Button,
            _: &[Modifier],
            _: u32,
        ) -> Result<(), vcli_input::error::InputError> {
            Ok(())
        }
        fn double_click(&self, _: Point, _: Button) -> Result<(), vcli_input::error::InputError> {
            Ok(())
        }
        fn drag(
            &self,
            _: Point,
            _: &[vcli_input::sink::DragSegment],
            _: Button,
        ) -> Result<(), vcli_input::error::InputError> {
            Ok(())
        }
        fn type_text(&self, _: &str) -> Result<(), vcli_input::error::InputError> {
            Ok(())
        }
        fn key_combo(&self, _: &[Modifier], _: &str) -> Result<(), vcli_input::error::InputError> {
            Ok(())
        }
    }

    #[test]
    fn absolute_click_advances() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let input: Arc<dyn InputSink> = Arc::new(NopSink);
        let mut st = BodyState::default();
        let body = vec![Step::Click {
            at: Target::Absolute(Point { x: 1, y: 2 }),
            button: Button::Left,
        }];
        let out = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 0, &assets, &p, &input);
        assert!(matches!(out, StepOutcome::Advanced));
    }

    #[test]
    fn sleep_ms_stalls_then_advances() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let input: Arc<dyn InputSink> = Arc::new(NopSink);
        let mut st = BodyState::default();
        let body = vec![Step::SleepMs { ms: 100 }];
        let out = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 0, &assets, &p, &input);
        assert!(matches!(out, StepOutcome::Stalled));
        let out2 = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 200, &assets, &p, &input);
        assert!(matches!(out2, StepOutcome::Advanced));
    }

    #[test]
    fn assert_fail_propagates_error() {
        use vcli_core::predicate::{PredicateKind, Rgb};
        let mut preds = BTreeMap::new();
        preds.insert(
            "blue".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([0, 0, 255]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let p = Perception::new();
        let input: Arc<dyn InputSink> = Arc::new(NopSink);
        let mut st = BodyState::default();
        let body = vec![Step::Assert {
            predicate: "blue".into(),
            on_fail: OnFail::Fail,
        }];
        let out = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 0, &assets, &p, &input);
        assert!(matches!(out, StepOutcome::Failed(RuntimeError::AssertFailed { .. })));
    }

    #[test]
    fn cursor_past_end_is_body_complete() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let input: Arc<dyn InputSink> = Arc::new(NopSink);
        let mut st = BodyState::default();
        let body: Vec<Step> = vec![];
        let out = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 0, &assets, &p, &input);
        assert!(matches!(out, StepOutcome::BodyComplete));
    }
}
