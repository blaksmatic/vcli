//! Error codes and wire payloads shared across the daemon, IPC, and CLI.
//!
//! `ErrorCode` is a stable enum — code strings are part of the IPC contract.
//! `ErrorPayload` mirrors the `{code, message, path?, line?, column?, span_len?}`
//! shape required by Decision 2.2.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable machine-readable error code. String form matches spec §IPC → Error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// DSL validation failed. Accompanied by JSON path.
    InvalidProgram,
    /// Program id not found.
    UnknownProgram,
    /// Illegal state transition, e.g. `cancel` on a completed program.
    BadStateTransition,
    /// macOS Accessibility or Screen Recording permission not granted.
    PermissionDenied,
    /// Capture backend error.
    CaptureFailed,
    /// Daemon is too busy / queue full.
    DaemonBusy,
    /// Resume rejected because program state disqualifies it (Decisions 2.4, C).
    NotResumable,
    /// `vcli resume`: the step N-1 postcondition no longer holds.
    ResumePreconditionFailed,
    /// Catch-all, logged server-side with correlation id.
    Internal,
}

impl ErrorCode {
    /// Wire string form (same as serde rename).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidProgram => "invalid_program",
            Self::UnknownProgram => "unknown_program",
            Self::BadStateTransition => "bad_state_transition",
            Self::PermissionDenied => "permission_denied",
            Self::CaptureFailed => "capture_failed",
            Self::DaemonBusy => "daemon_busy",
            Self::NotResumable => "not_resumable",
            Self::ResumePreconditionFailed => "resume_precondition_failed",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Wire-shape error returned on IPC responses. Decision 2.2 adds line/column/span for
/// parse/validation errors; other codes leave those `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    /// Stable code.
    pub code: ErrorCode,
    /// Human-readable message.
    pub message: String,
    /// JSON path into the offending program (e.g. `watches[0].when`), if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 1-based source line (for DSL parse errors).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based source column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Length of the offending span in characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_len: Option<u32>,
    /// Optional did-you-mean hint (Levenshtein-1 suggestion) — Decision 2.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ErrorPayload {
    /// Minimal constructor for non-DSL errors.
    #[must_use]
    pub fn simple(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
            line: None,
            column: None,
            span_len: None,
            hint: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_serializes_snake_case() {
        for (c, s) in [
            (ErrorCode::InvalidProgram, "invalid_program"),
            (ErrorCode::UnknownProgram, "unknown_program"),
            (ErrorCode::BadStateTransition, "bad_state_transition"),
            (ErrorCode::PermissionDenied, "permission_denied"),
            (ErrorCode::CaptureFailed, "capture_failed"),
            (ErrorCode::DaemonBusy, "daemon_busy"),
            (ErrorCode::NotResumable, "not_resumable"),
            (ErrorCode::ResumePreconditionFailed, "resume_precondition_failed"),
            (ErrorCode::Internal, "internal"),
        ] {
            assert_eq!(c.as_str(), s);
            let j = serde_json::to_string(&c).unwrap();
            assert_eq!(j, format!(r#""{s}""#));
            let back: ErrorCode = serde_json::from_str(&j).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn simple_payload_omits_optional_fields() {
        let p = ErrorPayload::simple(ErrorCode::UnknownProgram, "not found");
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""code":"unknown_program""#));
        assert!(!j.contains("path"));
        assert!(!j.contains("line"));
        assert!(!j.contains("column"));
        assert!(!j.contains("span_len"));
        assert!(!j.contains("hint"));
    }

    #[test]
    fn full_payload_roundtrip() {
        let p = ErrorPayload {
            code: ErrorCode::InvalidProgram,
            message: "unknown predicate 'skp_visible'".into(),
            path: Some("watches[0].when".into()),
            line: Some(12),
            column: Some(18),
            span_len: Some(12),
            hint: Some("skip_visible".into()),
        };
        let back: ErrorPayload = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}
