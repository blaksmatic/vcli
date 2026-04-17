//! `InputSink` — the synchronous, OS-confirmed input interface.
//!
//! Every method must return only after the OS has dispatched the event
//! (microseconds, not visual confirmation — the runtime layers postcondition
//! checks on top per spec §Action confirmation). `Result<(), InputError>`
//! means "the OS accepted the event" or "we failed before posting". A
//! `KillSwitch` engaged at entry produces `InputError::Halted` before any
//! OS call.

use std::time::Duration;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use crate::error::InputError;

/// One hop of a multi-point drag. The first segment begins at `from`; later
/// segments interpolate from the prior segment's `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragSegment {
    /// End point of this segment in logical pixels.
    pub to: Point,
    /// Duration of this segment. Backends emit interpolated move events over
    /// this span to mimic a human-speed drag.
    pub duration: Duration,
}

/// Synchronous input trait. Implementors must be `Send + Sync` so the runtime
/// can hold one behind an `Arc`.
pub trait InputSink: Send + Sync {
    /// Move the cursor to `to` immediately (no interpolation).
    ///
    /// # Errors
    ///
    /// Returns `InputError::Halted` if the kill switch is engaged, or a
    /// backend-specific error if the OS rejects the event.
    fn mouse_move(&self, to: Point) -> Result<(), InputError>;

    /// Click `button` at `at` with `modifiers` held. `hold_ms` is the down→up
    /// gap (0 = fire down+up back-to-back). Backends that cannot honor hold
    /// duration still must return `Ok(())` only after the up event posts.
    ///
    /// # Errors
    ///
    /// Returns `InputError::Halted` if the kill switch is engaged, or a
    /// backend-specific error if the OS rejects the event.
    fn click(
        &self,
        at: Point,
        button: Button,
        modifiers: &[Modifier],
        hold_ms: u32,
    ) -> Result<(), InputError>;

    /// Double-click `button` at `at`. Implementations set the
    /// `MouseEventClickState` field to 2 on the second press so the OS treats
    /// it as a double-click (not two single clicks).
    ///
    /// # Errors
    ///
    /// Returns `InputError::Halted` if the kill switch is engaged, or a
    /// backend-specific error if the OS rejects the event.
    fn double_click(&self, at: Point, button: Button) -> Result<(), InputError>;

    /// Drag from `from` through each `DragSegment.to` in order. `button` is
    /// held down across all segments and released at the final `to`. Returns
    /// only after the final mouse-up is posted.
    ///
    /// # Errors
    ///
    /// Returns `InputError::InvalidArgument` if `segments` is empty,
    /// `InputError::Halted` if the kill switch fires mid-drag, or a backend
    /// error if the OS rejects any event.
    fn drag(&self, from: Point, segments: &[DragSegment], button: Button)
        -> Result<(), InputError>;

    /// Type literal UTF-8 text. Backends use Unicode key events (macOS:
    /// `CGEventKeyboardSetUnicodeString`) so the active keyboard layout is
    /// respected and arbitrary code-points (including non-ASCII) type correctly.
    ///
    /// # Errors
    ///
    /// Returns `InputError::Halted` if the kill switch is engaged, or a
    /// backend-specific error if the OS rejects any key event.
    fn type_text(&self, text: &str) -> Result<(), InputError>;

    /// Press a key combo. `key` uses the vcli canonical key-name set
    /// (e.g. `"s"`, `"return"`, `"space"`, `"f1"`); see [`crate::keymap`].
    /// Down/up pairs are emitted for each modifier around the primary key.
    ///
    /// # Errors
    ///
    /// Returns `InputError::UnknownKey` if `key` is not in the canonical set,
    /// `InputError::Halted` if the kill switch is engaged, or a backend error
    /// if the OS rejects the event.
    fn key_combo(&self, modifiers: &[Modifier], key: &str) -> Result<(), InputError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal stand-in impl to prove the trait is object-safe and compiles.
    struct Nop;
    impl InputSink for Nop {
        fn mouse_move(&self, _: Point) -> Result<(), InputError> {
            Ok(())
        }
        fn click(&self, _: Point, _: Button, _: &[Modifier], _: u32) -> Result<(), InputError> {
            Ok(())
        }
        fn double_click(&self, _: Point, _: Button) -> Result<(), InputError> {
            Ok(())
        }
        fn drag(&self, _: Point, _: &[DragSegment], _: Button) -> Result<(), InputError> {
            Ok(())
        }
        fn type_text(&self, _: &str) -> Result<(), InputError> {
            Ok(())
        }
        fn key_combo(&self, _: &[Modifier], _: &str) -> Result<(), InputError> {
            Ok(())
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let s: Box<dyn InputSink> = Box::new(Nop);
        s.mouse_move(Point { x: 1, y: 2 }).unwrap();
    }

    #[test]
    fn drag_segment_is_copy() {
        let s = DragSegment {
            to: Point { x: 5, y: 5 },
            duration: Duration::from_millis(100),
        };
        let s2 = s;
        assert_eq!(s.to, s2.to);
    }
}
