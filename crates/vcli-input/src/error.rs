//! Errors surfaced by [`InputSink`] implementations.

use thiserror::Error;
use vcli_core::ErrorCode;

/// Error returned by every `InputSink` method. `Halted` wins over all others
/// because the kill switch short-circuits before the backend is even called.
#[derive(Debug, Error)]
pub enum InputError {
    /// The kill switch was set; no OS event was posted.
    #[error("input halted: kill switch is engaged")]
    Halted,

    /// macOS TCC (Accessibility / Input Monitoring) has not granted this process
    /// permission to synthesize input.
    #[error("input permission denied: {detail}")]
    PermissionDenied {
        /// Short human-readable reason — which TCC bucket is missing.
        detail: String,
    },

    /// Event creation / posting failed at the OS layer.
    #[error("backend failure: {detail}")]
    Backend {
        /// Error message from the OS / FFI call.
        detail: String,
    },

    /// Unknown key name in a key-combo action.
    #[error("unknown key: {0}")]
    UnknownKey(String),

    /// Invalid argument (e.g. empty drag path, `hold_ms > 60_000`).
    #[error("invalid input argument: {0}")]
    InvalidArgument(String),

    /// Platform does not support this method (Windows stub in v0).
    #[error("not implemented on this platform")]
    Unimplemented,
}

impl InputError {
    /// Map to the IPC wire-level [`ErrorCode`]. `Halted` maps to `internal`
    /// because it is a local-only condition with no external counterpart.
    #[must_use]
    pub fn to_error_code(&self) -> ErrorCode {
        match self {
            Self::Halted | Self::Backend { .. } | Self::Unimplemented => ErrorCode::Internal,
            Self::PermissionDenied { .. } => ErrorCode::PermissionDenied,
            Self::UnknownKey(_) | Self::InvalidArgument(_) => ErrorCode::InvalidProgram,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halted_display_is_stable() {
        assert_eq!(
            InputError::Halted.to_string(),
            "input halted: kill switch is engaged"
        );
    }

    #[test]
    fn permission_maps_to_permission_denied() {
        let e = InputError::PermissionDenied {
            detail: "Accessibility".into(),
        };
        assert_eq!(e.to_error_code(), ErrorCode::PermissionDenied);
    }

    #[test]
    fn unknown_key_maps_to_invalid_program() {
        let e = InputError::UnknownKey("blargh".into());
        assert_eq!(e.to_error_code(), ErrorCode::InvalidProgram);
        assert!(e.to_string().contains("blargh"));
    }

    #[test]
    fn halted_maps_to_internal() {
        assert_eq!(InputError::Halted.to_error_code(), ErrorCode::Internal);
    }
}
