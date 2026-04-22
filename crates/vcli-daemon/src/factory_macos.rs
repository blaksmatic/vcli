//! Real macOS `RuntimeBackends` factory. Compiled only on
//! `target_os = "macos"`. Constructs `MacCapture` + `CGEventInputSink`
//! and parks the kill-switch listener handle on `_shutdown_guard`.
//!
//! See spec Decisions B1, B2, B3, B5.

#![cfg(target_os = "macos")]

use std::sync::Arc;

use vcli_capture::macos::MacCapture;
use vcli_capture::permission::request_screen_recording_permission;
use vcli_input::macos::{spawn_kill_switch_listener, CGEventInputSink};
use vcli_input::KillSwitch;

use crate::error::{DaemonError, DaemonResult};
use crate::run::RuntimeBackends;

/// Build the production macOS `RuntimeBackends`.
///
/// Two of the three real-backend constructors are fallible:
/// - `MacCapture::new()` returns `Result<Self, CaptureError>` (TCC probe).
/// - `spawn_kill_switch_listener()` returns `Result<KillSwitchListenerHandle, InputError>`.
/// - `CGEventInputSink::new(kill)` is infallible (`-> Self`).
///
/// # Errors
///
/// `DaemonError::BackendInit { backend: "capture", .. }` if Screen Recording
/// is not granted (TCC denial in `MacCapture::new`).
///
/// `DaemonError::BackendInit { backend: "input", .. }` if the kill-switch
/// listener thread cannot be spawned (typically Input Monitoring is denied).
pub fn build() -> DaemonResult<RuntimeBackends> {
    // Trigger the macOS Screen Recording prompt the first time an
    // unprivileged binary attempts to construct MacCapture. Without this,
    // CGPreflightScreenCaptureAccess (used inside MacCapture::new) returns
    // false silently — the daemon fails clean per Decision B5 but the user
    // never sees a system dialog and has to manually add the binary in
    // System Settings → Privacy & Security → Screen Recording. Calling
    // CGRequestScreenCaptureAccess here is async from the user's
    // perspective: the dialog opens, but MacCapture::new below will still
    // observe Denied on this run. Restart picks up the granted state.
    // Ignore the result — request can't fail in any actionable way.
    let _ = request_screen_recording_permission();

    let capture = MacCapture::new().map_err(|e| DaemonError::BackendInit {
        backend: "capture",
        reason: format!(
            "{e} — grant access in System Settings → Privacy & Security → Screen Recording, then restart the daemon"
        ),
    })?;

    let kill = KillSwitch::new();
    let listener = spawn_kill_switch_listener(kill.clone()).map_err(|e| {
        DaemonError::BackendInit {
            backend: "input",
            reason: format!(
                "kill-switch listener: {e} — grant access in System Settings → Privacy & Security → Input Monitoring, then restart the daemon"
            ),
        }
    })?;
    // CGEventInputSink::new is infallible — no `?` here.
    let input = CGEventInputSink::new(kill);

    Ok(RuntimeBackends {
        capture: Box::new(capture),
        input: Arc::new(input),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_runtime::SystemRuntimeClock::new()),
        _shutdown_guard: Some(Box::new(listener)),
    })
}

#[cfg(test)]
mod tests {
    // build() touches macOS TCC and may prompt the user. Tests live in
    // `tests/real_backends_macos.rs` and are #[ignore]d.
}
