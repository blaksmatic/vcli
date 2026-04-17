//! Real CGEvent smoke tests. Gated with `#[ignore]` because they move the
//! cursor and need TCC Accessibility (+ optionally Input Monitoring) granted.
//!
//! Run manually with:
//!     cargo test -p vcli-input --test macos_real -- --ignored
//!
//! If Accessibility is not granted, every test will produce
//! `InputError::PermissionDenied` and fail fast with a clear message.

#![cfg(target_os = "macos")]

use std::time::Duration;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use vcli_input::kill_switch::KillSwitch;
use vcli_input::macos::CGEventInputSink;
use vcli_input::permissions::{probe, PermissionStatus};
use vcli_input::sink::{DragSegment, InputSink};

fn require_accessibility() -> Result<(), String> {
    let r = probe();
    if matches!(r.accessibility, PermissionStatus::Granted) {
        Ok(())
    } else {
        Err(format!("Accessibility TCC not granted: {r:?}"))
    }
}

fn sink() -> CGEventInputSink {
    CGEventInputSink::new(KillSwitch::new())
}

#[test]
#[ignore = "moves the cursor; requires TCC Accessibility"]
fn mouse_move_to_origin_and_back() {
    require_accessibility().unwrap();
    let s = sink();
    s.mouse_move(Point { x: 50, y: 50 }).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    s.mouse_move(Point { x: 200, y: 200 }).unwrap();
}

#[test]
#[ignore = "clicks the screen; requires TCC Accessibility"]
fn click_left_at_a_safe_spot() {
    require_accessibility().unwrap();
    let s = sink();
    // (10, 10) is usually over the menu bar / desktop; harmless.
    s.click(Point { x: 10, y: 10 }, Button::Left, &[], 20)
        .unwrap();
}

#[test]
#[ignore = "types into the focused window; requires TCC Accessibility"]
fn type_ascii_text() {
    require_accessibility().unwrap();
    let s = sink();
    s.type_text("hello").unwrap();
}

#[test]
#[ignore = "presses Cmd+A in the focused window; requires TCC Accessibility"]
fn key_combo_cmd_a() {
    require_accessibility().unwrap();
    let s = sink();
    s.key_combo(&[Modifier::Cmd], "a").unwrap();
}

#[test]
#[ignore = "drags across 100 pixels; requires TCC Accessibility"]
fn drag_100_pixels() {
    require_accessibility().unwrap();
    let s = sink();
    s.drag(
        Point { x: 100, y: 100 },
        &[DragSegment {
            to: Point { x: 200, y: 200 },
            duration: Duration::from_millis(200),
        }],
        Button::Left,
    )
    .unwrap();
}

#[test]
#[ignore = "verifies kill switch engagement short-circuits real sink"]
fn kill_switch_short_circuits_real_sink() {
    require_accessibility().unwrap();
    let kill = KillSwitch::new();
    let s = CGEventInputSink::new(kill.clone());
    kill.engage();
    let e = s.mouse_move(Point { x: 100, y: 100 }).unwrap_err();
    assert!(matches!(e, vcli_input::error::InputError::Halted));
}
