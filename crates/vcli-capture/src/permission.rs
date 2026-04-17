//! Screen Recording (TCC) permission probe.
//!
//! On macOS this calls `CGPreflightScreenCaptureAccess` and
//! `CGRequestScreenCaptureAccess` from the Core Graphics framework directly
//! via `extern "C"` links. On other OSes the result is always `Granted`
//! because there is no equivalent gating.
//!
//! Note: `screencapturekit` v1.5.4 (the actual published version; plan cited
//! "0.3" which does not exist on crates.io) does not expose a
//! `util::has_permission` helper. We link the Core Graphics symbols directly
//! instead, which is the idiomatic macOS approach.
#![allow(unsafe_code)] // Required for CGPreflightScreenCaptureAccess / CGRequestScreenCaptureAccess extern "C" links on macOS.

use crate::error::CaptureError;

/// Result of probing the OS for screen-recording permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    /// Permission granted — capture will succeed.
    Granted,
    /// Not yet granted. On macOS the daemon should emit
    /// `capture.permission_missing { backend: "macos" }` per spec §Events.
    Denied,
    /// Backend is unable to determine the status (e.g., running under `nohup`
    /// with no window server). Treated as denied by the scheduler but
    /// surfaces to the user as a more specific diagnostic.
    Unknown,
}

/// Synchronously probe the OS for screen-recording permission.
///
/// - macOS: calls `CGPreflightScreenCaptureAccess()`. Does not prompt.
///   Use `request_screen_recording_permission()` to trigger the system prompt.
/// - Other OSes: always returns `Granted`.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] only if the underlying FFI call fails
/// in a way that is not "permission denied" (very rare — e.g., window server
/// unreachable).
pub fn check_screen_recording_permission() -> Result<PermissionStatus, CaptureError> {
    cfg_if_macos()
}

#[cfg(target_os = "macos")]
#[allow(clippy::unnecessary_wraps)]
fn cfg_if_macos() -> Result<PermissionStatus, CaptureError> {
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    // SAFETY: CGPreflightScreenCaptureAccess is a stable macOS framework
    // symbol with no preconditions beyond the process having a window
    // server connection. It is safe to call from any thread.
    let granted = unsafe { CGPreflightScreenCaptureAccess() };
    Ok(if granted {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    })
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::unnecessary_wraps)]
fn cfg_if_macos() -> Result<PermissionStatus, CaptureError> {
    Ok(PermissionStatus::Granted)
}

/// Request the user grant screen-recording permission. Triggers the macOS
/// system prompt if currently Denied. No-op on other OSes.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] if the request call fails.
pub fn request_screen_recording_permission() -> Result<(), CaptureError> {
    do_request()
}

#[cfg(target_os = "macos")]
#[allow(clippy::unnecessary_wraps)]
fn do_request() -> Result<(), CaptureError> {
    extern "C" {
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    // SAFETY: CGRequestScreenCaptureAccess is a stable macOS framework
    // symbol. We ignore the return value — the prompt is async from the
    // user's perspective; the caller should re-probe after user action.
    let _granted = unsafe { CGRequestScreenCaptureAccess() };
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::unnecessary_wraps)]
fn do_request() -> Result<(), CaptureError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_status_is_copy_eq() {
        let a = PermissionStatus::Granted;
        let b = a;
        assert_eq!(a, b);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_always_granted() {
        let s = check_screen_recording_permission().unwrap();
        assert_eq!(s, PermissionStatus::Granted);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_request_is_no_op() {
        request_screen_recording_permission().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_probe_does_not_panic() {
        // Doesn't assert Granted/Denied — that's environment-dependent.
        // Just ensures the FFI call wiring works without unwind.
        let _ = check_screen_recording_permission().unwrap();
    }
}
