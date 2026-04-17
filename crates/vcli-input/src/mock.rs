//! `MockInputSink` — records every call as an [`InputAction`] for assertions
//! in downstream crates' scenario tests. Also honors the [`KillSwitch`] so
//! kill-switch semantics are testable without an OS backend.

use std::sync::Mutex;
use std::time::Duration;

use vcli_core::action::{Button, InputAction, Modifier};
use vcli_core::geom::Point;

use crate::error::InputError;
use crate::kill_switch::KillSwitch;
use crate::sink::{DragSegment, InputSink};

/// An entry in the mock call log. Mostly 1-to-1 with [`InputAction`], but adds
/// variants the DSL-level action enum doesn't carry (`DoubleClick`, `Drag`).
#[derive(Debug, Clone, PartialEq)]
pub enum MockCall {
    /// Low-level action the runtime would emit.
    Action(InputAction),
    /// Double-click at a point with a button.
    DoubleClick {
        /// Point.
        at: Point,
        /// Button.
        button: Button,
    },
    /// Drag from `from` through segment endpoints with `button` held.
    Drag {
        /// Start point.
        from: Point,
        /// Endpoints (duration omitted — mock does not sleep).
        to: Vec<Point>,
        /// Held button.
        button: Button,
    },
    /// A click variant that carries modifiers + hold (`InputAction::Click` drops them).
    ClickDetailed {
        /// Point.
        at: Point,
        /// Button.
        button: Button,
        /// Modifiers.
        modifiers: Vec<Modifier>,
        /// Hold-down time in ms.
        hold_ms: u32,
    },
}

/// Recording `InputSink`. Thread-safe.
#[derive(Debug, Default)]
pub struct MockInputSink {
    log: Mutex<Vec<MockCall>>,
    kill: KillSwitch,
    /// Optional artificial error; when set, all methods return this. Useful to
    /// exercise the runtime's error-handling paths.
    forced_error: Mutex<Option<String>>,
}

impl MockInputSink {
    /// Fresh mock with an unengaged kill switch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build with a caller-provided kill switch so tests can trigger `Halted`.
    #[must_use]
    pub fn with_kill_switch(kill: KillSwitch) -> Self {
        Self {
            log: Mutex::new(Vec::new()),
            kill,
            forced_error: Mutex::new(None),
        }
    }

    /// Read the current call log.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned (should not happen in normal use).
    #[must_use]
    pub fn calls(&self) -> Vec<MockCall> {
        self.log.lock().unwrap().clone()
    }

    /// Drain and return calls (clears the log).
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned (should not happen in normal use).
    pub fn drain(&self) -> Vec<MockCall> {
        std::mem::take(&mut *self.log.lock().unwrap())
    }

    /// Reference to the underlying kill switch (clone to share).
    #[must_use]
    pub fn kill_switch(&self) -> KillSwitch {
        self.kill.clone()
    }

    /// Force every subsequent call to fail with `InputError::Backend { detail }`.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned (should not happen in normal use).
    pub fn fail_with(&self, detail: impl Into<String>) {
        *self.forced_error.lock().unwrap() = Some(detail.into());
    }

    fn check(&self) -> Result<(), InputError> {
        if self.kill.is_engaged() {
            return Err(InputError::Halted);
        }
        if let Some(d) = self.forced_error.lock().unwrap().clone() {
            return Err(InputError::Backend { detail: d });
        }
        Ok(())
    }

    fn push(&self, c: MockCall) {
        self.log.lock().unwrap().push(c);
    }
}

impl InputSink for MockInputSink {
    fn mouse_move(&self, to: Point) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::Action(InputAction::Move { at: to }));
        Ok(())
    }

    fn click(
        &self,
        at: Point,
        button: Button,
        modifiers: &[Modifier],
        hold_ms: u32,
    ) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::ClickDetailed {
            at,
            button,
            modifiers: modifiers.to_vec(),
            hold_ms,
        });
        Ok(())
    }

    fn double_click(&self, at: Point, button: Button) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::DoubleClick { at, button });
        Ok(())
    }

    fn drag(
        &self,
        from: Point,
        segments: &[DragSegment],
        button: Button,
    ) -> Result<(), InputError> {
        self.check()?;
        if segments.is_empty() {
            return Err(InputError::InvalidArgument(
                "drag segments must be non-empty".into(),
            ));
        }
        self.push(MockCall::Drag {
            from,
            to: segments.iter().map(|s| s.to).collect(),
            button,
        });
        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::Action(InputAction::Type {
            text: text.to_owned(),
        }));
        Ok(())
    }

    fn key_combo(&self, modifiers: &[Modifier], key: &str) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::Action(InputAction::Key {
            key: key.to_owned(),
            modifiers: modifiers.to_vec(),
        }));
        Ok(())
    }
}

// Silence unused-import warning when Duration isn't used after refactor.
#[allow(dead_code)]
fn _duration_in_scope(_: Duration) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_records_move() {
        let m = MockInputSink::new();
        m.mouse_move(Point { x: 3, y: 4 }).unwrap();
        assert_eq!(
            m.calls(),
            vec![MockCall::Action(InputAction::Move {
                at: Point { x: 3, y: 4 }
            })]
        );
    }

    #[test]
    fn mock_records_click_with_modifiers() {
        let m = MockInputSink::new();
        m.click(Point { x: 1, y: 1 }, Button::Left, &[Modifier::Cmd], 120)
            .unwrap();
        assert_eq!(
            m.calls(),
            vec![MockCall::ClickDetailed {
                at: Point { x: 1, y: 1 },
                button: Button::Left,
                modifiers: vec![Modifier::Cmd],
                hold_ms: 120,
            }]
        );
    }

    #[test]
    fn mock_records_double_click_and_drag() {
        let m = MockInputSink::new();
        m.double_click(Point { x: 0, y: 0 }, Button::Left).unwrap();
        m.drag(
            Point { x: 0, y: 0 },
            &[DragSegment {
                to: Point { x: 10, y: 10 },
                duration: Duration::from_millis(50),
            }],
            Button::Left,
        )
        .unwrap();
        let calls = m.calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], MockCall::DoubleClick { .. }));
        assert!(matches!(calls[1], MockCall::Drag { .. }));
    }

    #[test]
    fn mock_records_type_and_key() {
        let m = MockInputSink::new();
        m.type_text("hi").unwrap();
        m.key_combo(&[Modifier::Cmd, Modifier::Shift], "s").unwrap();
        assert_eq!(
            m.calls(),
            vec![
                MockCall::Action(InputAction::Type { text: "hi".into() }),
                MockCall::Action(InputAction::Key {
                    key: "s".into(),
                    modifiers: vec![Modifier::Cmd, Modifier::Shift],
                }),
            ]
        );
    }

    #[test]
    fn mock_rejects_empty_drag() {
        let m = MockInputSink::new();
        let e = m.drag(Point { x: 0, y: 0 }, &[], Button::Left).unwrap_err();
        matches!(e, InputError::InvalidArgument(_));
    }

    #[test]
    fn drain_empties_the_log() {
        let m = MockInputSink::new();
        m.mouse_move(Point { x: 0, y: 0 }).unwrap();
        let first = m.drain();
        assert_eq!(first.len(), 1);
        assert!(m.calls().is_empty());
    }

    #[test]
    fn forced_error_produces_backend_failure() {
        let m = MockInputSink::new();
        m.fail_with("flaky");
        let e = m.mouse_move(Point { x: 0, y: 0 }).unwrap_err();
        assert!(matches!(e, InputError::Backend { .. }));
        assert!(m.calls().is_empty(), "failed call must not be recorded");
    }
}
