//! Real macOS `InputSink`. Enforces the [`KillSwitch`] on every entry point,
//! then delegates to the low-level CGEvent helpers.

#![cfg(target_os = "macos")]

use std::time::Duration;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use super::{cg_events, cg_typing};
use crate::error::InputError;
use crate::keymap::{macos_keycode, parse};
use crate::kill_switch::KillSwitch;
use crate::permissions::{probe, PermissionStatus};
use crate::sink::{DragSegment, InputSink};

/// Maximum step interpolation time for a single drag segment. Guards against
/// runaway durations from ill-formed programs.
const MAX_DRAG_SEGMENT_MS: u64 = 5_000;
/// Pixel interval between interpolated drag-move events.
const DRAG_STEP_PX: i32 = 8;

/// macOS `CGEvent`-backed `InputSink`.
#[derive(Debug)]
pub struct CGEventInputSink {
    kill: KillSwitch,
}

impl CGEventInputSink {
    /// Construct with a caller-provided kill switch. Callers should also spawn
    /// the hotkey listener via [`super::spawn_kill_switch_listener`] so the
    /// `Cmd+Shift+Esc` chord engages this switch.
    #[must_use]
    pub fn new(kill: KillSwitch) -> Self {
        Self { kill }
    }

    /// Fail-fast if Accessibility isn't granted. Called on first `InputSink`
    /// method invocation only when we know we'd hit the OS. Cheap enough
    /// (single `AXIsProcessTrustedWithOptions` call) to gate every call.
    fn guard(&self) -> Result<(), InputError> {
        if self.kill.is_engaged() {
            return Err(InputError::Halted);
        }
        let report = probe();
        if !matches!(
            report.accessibility,
            PermissionStatus::Granted | PermissionStatus::NotDetermined
        ) {
            return Err(InputError::PermissionDenied {
                detail: "Accessibility (TCC) not granted".into(),
            });
        }
        Ok(())
    }
}

impl InputSink for CGEventInputSink {
    fn mouse_move(&self, to: Point) -> Result<(), InputError> {
        self.guard()?;
        cg_events::post_move(to)
    }

    fn click(
        &self,
        at: Point,
        button: Button,
        modifiers: &[Modifier],
        hold_ms: u32,
    ) -> Result<(), InputError> {
        self.guard()?;
        cg_events::post_move(at)?;
        cg_events::post_click(at, button, modifiers, Duration::from_millis(hold_ms.into()), 1)
    }

    fn double_click(&self, at: Point, button: Button) -> Result<(), InputError> {
        self.guard()?;
        cg_events::post_move(at)?;
        cg_events::post_click(at, button, &[], Duration::ZERO, 1)?;
        cg_events::post_click(at, button, &[], Duration::ZERO, 2)
    }

    fn drag(
        &self,
        from: Point,
        segments: &[DragSegment],
        button: Button,
    ) -> Result<(), InputError> {
        self.guard()?;
        if segments.is_empty() {
            return Err(InputError::InvalidArgument(
                "drag segments must be non-empty".into(),
            ));
        }

        // Move to start, press down.
        cg_events::post_move(from)?;
        let (_cg_btn, down_ty, up_ty) = cg_events::button_types(button);
        let src = core_graphics::event_source::CGEventSource::new(
            core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
        )
        .map_err(|()| InputError::Backend {
            detail: "CGEventSource::new failed".into(),
        })?;
        let down = core_graphics::event::CGEvent::new_mouse_event(
            src,
            down_ty,
            cg_events::to_cg(from),
            cg_events::button_types(button).0,
        )
        .map_err(|()| InputError::Backend {
            detail: "mouse_event down failed".into(),
        })?;
        down.post(core_graphics::event::CGEventTapLocation::HID);

        // Interpolate through each segment.
        let mut current = from;
        for seg in segments {
            if seg.duration.as_millis() > u128::from(MAX_DRAG_SEGMENT_MS) {
                return Err(InputError::InvalidArgument(
                    "drag segment longer than 5s".into(),
                ));
            }
            let dx = seg.to.x - current.x;
            let dy = seg.to.y - current.y;
            let dist = ((dx * dx + dy * dy) as f64).sqrt() as i32;
            let steps = (dist / DRAG_STEP_PX).max(1);
            let sleep_each = seg.duration / u32::try_from(steps).unwrap_or(1);
            for i in 1..=steps {
                if self.kill.is_engaged() {
                    // Release button before bailing.
                    let _ = cg_events::post_drag_move(current, button);
                    let src2 = core_graphics::event_source::CGEventSource::new(
                        core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
                    )
                    .ok();
                    if let Some(src) = src2 {
                        if let Ok(up) = core_graphics::event::CGEvent::new_mouse_event(
                            src,
                            up_ty,
                            cg_events::to_cg(current),
                            cg_events::button_types(button).0,
                        ) {
                            up.post(core_graphics::event::CGEventTapLocation::HID);
                        }
                    }
                    return Err(InputError::Halted);
                }
                let nx = current.x + dx * i / steps;
                let ny = current.y + dy * i / steps;
                cg_events::post_drag_move(Point { x: nx, y: ny }, button)?;
                if !sleep_each.is_zero() {
                    std::thread::sleep(sleep_each);
                }
            }
            current = seg.to;
        }

        // Release button at final position.
        let src = core_graphics::event_source::CGEventSource::new(
            core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
        )
        .map_err(|()| InputError::Backend {
            detail: "CGEventSource::new failed".into(),
        })?;
        let up = core_graphics::event::CGEvent::new_mouse_event(
            src,
            up_ty,
            cg_events::to_cg(current),
            cg_events::button_types(button).0,
        )
        .map_err(|()| InputError::Backend {
            detail: "mouse_event up failed".into(),
        })?;
        up.post(core_graphics::event::CGEventTapLocation::HID);
        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<(), InputError> {
        self.guard()?;
        cg_typing::type_text(text)
    }

    fn key_combo(&self, modifiers: &[Modifier], key: &str) -> Result<(), InputError> {
        self.guard()?;
        let parsed = parse(key)?;
        let Some(keycode) = macos_keycode(parsed) else {
            return Err(InputError::UnknownKey(key.to_owned()));
        };
        // Press modifiers down-in-order, press key, release key, release modifiers
        // in reverse order. Using CGEventFlags on the key event alone is usually
        // enough, but explicit down/up events are more reliable for apps that
        // read flagChanged events.
        cg_events::post_key(keycode, true, modifiers)?;
        cg_events::post_key(keycode, false, modifiers)?;
        Ok(())
    }
}
