//! Screen Recording (TCC) permission probe — stub, filled out in Task 4.

use crate::error::CaptureError;

/// Result of probing the OS for screen-recording permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    /// Permission granted — capture will succeed.
    Granted,
    /// Not yet granted.
    Denied,
    /// Backend is unable to determine the status.
    Unknown,
}

/// Synchronously probe the OS for screen-recording permission.
///
/// # Errors
///
/// Returns [`CaptureError`] if the underlying FFI call fails.
pub fn check_screen_recording_permission() -> Result<PermissionStatus, CaptureError> {
    Ok(PermissionStatus::Granted)
}

/// Request the user grant screen-recording permission.
///
/// # Errors
///
/// Returns [`CaptureError`] if the request call fails.
pub fn request_screen_recording_permission() -> Result<(), CaptureError> {
    Ok(())
}
