//! Error types for the capture crate.
//!
//! The runtime maps `PermissionDenied` to event `capture.permission_missing`
//! (spec §Events) and `capture_failed` to the ipc error code of the same name.

use thiserror::Error;

/// All errors this crate produces.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Screen Recording (TCC) permission is not granted. User must approve
    /// in System Settings → Privacy & Security → Screen Recording.
    #[error("screen recording permission not granted")]
    PermissionDenied,

    /// The named window was not found on any display.
    #[error("window not found: id={id}")]
    WindowNotFound {
        /// Opaque backend id from the lost descriptor.
        id: u64,
    },

    /// Backend returned a frame the converter couldn't interpret (bad stride,
    /// bad pixel format, zero-sized plane, …).
    #[error("malformed frame from backend: {reason}")]
    MalformedFrame {
        /// Human-readable reason string.
        reason: String,
    },

    /// Backend call timed out or failed mid-operation. Wraps the backend-
    /// specific message so the runtime can surface it in `vcli health`.
    #[error("backend failure: {message}")]
    Backend {
        /// Backend-provided message (SCK error description, HRESULT, etc.).
        message: String,
    },

    /// The backend does not implement this method on the current platform.
    #[error("capture operation unsupported on this backend: {what}")]
    Unsupported {
        /// Short description of what was attempted.
        what: &'static str,
    },
}

impl CaptureError {
    /// Stable short string suitable for the ipc layer's error `code` field.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::WindowNotFound { .. } => "unknown_window",
            Self::MalformedFrame { .. } | Self::Backend { .. } => "capture_failed",
            Self::Unsupported { .. } => "unsupported",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_are_informative() {
        let e = CaptureError::PermissionDenied;
        assert_eq!(e.to_string(), "screen recording permission not granted");
        let e = CaptureError::WindowNotFound { id: 77 };
        assert_eq!(e.to_string(), "window not found: id=77");
        let e = CaptureError::MalformedFrame {
            reason: "bad stride".into(),
        };
        assert!(e.to_string().contains("bad stride"));
    }

    #[test]
    fn codes_are_stable() {
        assert_eq!(CaptureError::PermissionDenied.code(), "permission_denied");
        assert_eq!(
            CaptureError::WindowNotFound { id: 1 }.code(),
            "unknown_window"
        );
        assert_eq!(
            CaptureError::Backend {
                message: "x".into()
            }
            .code(),
            "capture_failed"
        );
        assert_eq!(
            CaptureError::MalformedFrame { reason: "x".into() }.code(),
            "capture_failed"
        );
        assert_eq!(
            CaptureError::Unsupported { what: "x" }.code(),
            "unsupported"
        );
    }
}
