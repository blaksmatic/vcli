//! Real-backend integration test, macOS only, #[ignore]d.
//!
//! Run with: `cargo test -p vcli-daemon --test real_backends_macos -- --ignored`
//!
//! Requires Screen Recording AND Input Monitoring granted to the test
//! binary. First run will trigger TCC prompts; grant them, then re-run.

#![cfg(target_os = "macos")]

use std::time::Duration;

#[test]
#[ignore = "requires Screen Recording + Input Monitoring TCC grants"]
fn factory_macos_build_yields_working_capture_and_drops_cleanly() {
    let mut backends = vcli_daemon::build_default_backends()
        .expect("build_default_backends — did you grant Screen Recording?");

    // Confirm capture actually works: grab one frame, expect non-zero size.
    let frame = backends
        .capture
        .grab_screen()
        .expect("MacCapture::grab_screen failed — Screen Recording probably denied");
    assert!(
        frame.bounds.w > 0 && frame.bounds.h > 0,
        "frame has zero dimensions: {:?}",
        frame.bounds
    );

    // The kill-switch listener thread should be alive; the only way to
    // observe it externally is to confirm Drop is clean. Park the bundle
    // briefly, then drop it.
    std::thread::sleep(Duration::from_millis(50));
    drop(backends);

    // If the listener thread didn't park its CFRunLoop properly we'd hang
    // here; the test's wall clock catches that.
    std::thread::sleep(Duration::from_millis(50));
}
