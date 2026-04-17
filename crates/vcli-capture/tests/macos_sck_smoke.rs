//! macOS-only smoke test that actually touches ScreenCaptureKit.
//!
//! Requires Screen Recording permission for the running process. Gated
//! behind `#[ignore]` so `cargo test` does not run it by default. Enable
//! manually with:  `cargo test -p vcli-capture --test macos_sck_smoke -- --ignored`

#![cfg(target_os = "macos")]

use vcli_capture::{
    capture::Capture,
    macos::MacCapture,
    permission::{check_screen_recording_permission, PermissionStatus},
};

#[test]
#[ignore]
fn enumerates_at_least_one_window() {
    assert_eq!(
        check_screen_recording_permission().unwrap(),
        PermissionStatus::Granted,
        "grant Screen Recording permission to the binary before running"
    );
    let c = MacCapture::new().expect("construct MacCapture");
    let windows = c.enumerate_windows().expect("enumerate");
    // A running GUI macOS session always has the Dock / menubar process, so
    // at least one window is guaranteed.
    assert!(!windows.is_empty(), "expected at least one window");
}

#[test]
#[ignore]
fn grabs_a_nonempty_screen_frame() {
    assert_eq!(
        check_screen_recording_permission().unwrap(),
        PermissionStatus::Granted
    );
    let mut c = MacCapture::new().expect("construct MacCapture");
    let frame = c.grab_screen().expect("grab_screen");
    assert!(frame.width() > 0);
    assert!(frame.height() > 0);
    assert!(!frame.pixels.is_empty());
    assert!(
        frame.pixels.iter().any(|b| *b != 0),
        "captured a fully-black frame — usually means TCC is silently denying"
    );
}
