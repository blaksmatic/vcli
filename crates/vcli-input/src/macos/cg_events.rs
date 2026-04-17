//! Raw `CGEvent` builders. Each function creates an event, posts it to the HID
//! tap, and returns. `CGEventPost` is synchronous — by the time it returns,
//! the OS has enqueued the event on the global event stream (spec §Action
//! confirmation).

#![allow(unsafe_code)]

use std::time::Duration;

use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGKeyCode, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use crate::error::InputError;

/// Make a new `CGEventSource` for `HIDSystemState`.
///
/// # Errors
///
/// Returns `InputError::Backend` if the `CGEventSource` FFI call fails.
fn event_source() -> Result<CGEventSource, InputError> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|()| InputError::Backend {
        detail: "CGEventSource::new failed".into(),
    })
}

/// Convert a `Point` to `CGPoint` in logical display coordinates.
#[must_use]
pub fn to_cg(p: Point) -> CGPoint {
    CGPoint::new(f64::from(p.x), f64::from(p.y))
}

/// Translate modifiers to a `CGEventFlags` bitmask.
#[must_use]
pub fn flags_from_modifiers(modifiers: &[Modifier]) -> CGEventFlags {
    let mut f = CGEventFlags::empty();
    for m in modifiers {
        f |= match m {
            Modifier::Cmd => CGEventFlags::CGEventFlagCommand,
            Modifier::Shift => CGEventFlags::CGEventFlagShift,
            Modifier::Alt => CGEventFlags::CGEventFlagAlternate,
            Modifier::Ctrl => CGEventFlags::CGEventFlagControl,
        };
    }
    f
}

/// Translate our `Button` to `CGMouseButton` + the (`type_down`, `type_up`) pair we
/// need to post for that button.
#[must_use]
pub fn button_types(b: Button) -> (CGMouseButton, CGEventType, CGEventType) {
    match b {
        Button::Left => (
            CGMouseButton::Left,
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
        ),
        Button::Right => (
            CGMouseButton::Right,
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
        ),
        Button::Middle => (
            CGMouseButton::Center,
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
        ),
    }
}

/// Post a `MouseMoved` event to the HID tap.
///
/// # Errors
///
/// Returns `InputError::Backend` if event creation or the OS call fails.
pub fn post_move(to: Point) -> Result<(), InputError> {
    let src = event_source()?;
    let event = CGEvent::new_mouse_event(
        src,
        CGEventType::MouseMoved,
        to_cg(to),
        CGMouseButton::Left, // ignored for MouseMoved
    )
    .map_err(|()| InputError::Backend {
        detail: "CGEvent::new_mouse_event MouseMoved failed".into(),
    })?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

/// Post a down-then-up click, setting `click_state` (`MouseEventClickState`
/// field, integer 1 for single-click, 2 for double-click, etc.) and modifier
/// flags.
///
/// # Errors
///
/// Returns `InputError::Backend` if event creation or the OS call fails.
pub fn post_click(
    at: Point,
    button: Button,
    modifiers: &[Modifier],
    hold: Duration,
    click_state: i64,
) -> Result<(), InputError> {
    let src = event_source()?;
    let (cg_btn, down_ty, up_ty) = button_types(button);
    let flags = flags_from_modifiers(modifiers);

    // Down.
    let down = CGEvent::new_mouse_event(src.clone(), down_ty, to_cg(at), cg_btn).map_err(|()| {
        InputError::Backend {
            detail: "mouse_event down failed".into(),
        }
    })?;
    down.set_flags(flags);
    down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    down.post(CGEventTapLocation::HID);

    if !hold.is_zero() {
        std::thread::sleep(hold);
    }

    // Up.
    let up = CGEvent::new_mouse_event(src, up_ty, to_cg(at), cg_btn).map_err(|()| {
        InputError::Backend {
            detail: "mouse_event up failed".into(),
        }
    })?;
    up.set_flags(flags);
    up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

/// Post a dragged mouse motion (button already down) to `to`.
///
/// # Errors
///
/// Returns `InputError::Backend` if event creation or the OS call fails.
pub fn post_drag_move(to: Point, button: Button) -> Result<(), InputError> {
    let src = event_source()?;
    let drag_ty = match button {
        Button::Left => CGEventType::LeftMouseDragged,
        Button::Right => CGEventType::RightMouseDragged,
        Button::Middle => CGEventType::OtherMouseDragged,
    };
    let cg_btn = button_types(button).0;
    let ev = CGEvent::new_mouse_event(src, drag_ty, to_cg(to), cg_btn).map_err(|()| {
        InputError::Backend {
            detail: "mouse_event drag failed".into(),
        }
    })?;
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

/// Post a key down or up event for a virtual keycode.
///
/// # Errors
///
/// Returns `InputError::Backend` if event creation or the OS call fails.
pub fn post_key(
    keycode: CGKeyCode,
    key_down: bool,
    modifiers: &[Modifier],
) -> Result<(), InputError> {
    let src = event_source()?;
    let ev =
        CGEvent::new_keyboard_event(src, keycode, key_down).map_err(|()| InputError::Backend {
            detail: "CGEvent::new_keyboard_event failed".into(),
        })?;
    ev.set_flags(flags_from_modifiers(modifiers));
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

/// Post a scroll-wheel event with pixel units.
///
/// # Errors
///
/// Returns `InputError::Backend` if event creation or the OS call fails.
pub fn post_scroll(_at: Point, dx: i32, dy: i32) -> Result<(), InputError> {
    // Scroll events aren't positional on macOS (they go to the focused window),
    // but we keep the same signature as `InputSink` for symmetry.
    let src = event_source()?;
    let ev =
        CGEvent::new_scroll_event(src, ScrollEventUnit::PIXEL, 2, dy, dx, 0).map_err(|()| {
            InputError::Backend {
                detail: "CGEvent::new_scroll_event failed".into(),
            }
        })?;
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

// No tests here: every function talks to the OS. Exercised by
// `tests/macos_real.rs` behind `#[ignore]`.
