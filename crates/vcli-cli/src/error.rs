//! CLI-layer errors. Every variant knows its spec-mandated exit code
//! (§589: `0` success, `1` generic, `2` validation, `3` not found, `4` daemon
//! not running). `CliError` bubbles up to `run()` and then to `main` as an
//! `i32`.

use std::io;

use thiserror::Error;

use vcli_core::{ErrorCode, ErrorPayload};
use vcli_ipc::IpcError;

/// Process exit code. `i32` repr matches the kernel's view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    /// `0` — command completed without error.
    Success = 0,
    /// `1` — generic error, human-readable message already printed.
    Generic = 1,
    /// `2` — DSL validation failed before or after submission.
    Validation = 2,
    /// `3` — program id not found / nothing matches.
    NotFound = 3,
    /// `4` — could not reach the daemon socket.
    DaemonDown = 4,
}

impl From<ExitCode> for i32 {
    fn from(c: ExitCode) -> Self {
        c as Self
    }
}

/// CLI-visible errors. Display impl is the user-facing message; `exit_code()`
/// is what `run()` returns.
#[derive(Debug, Error)]
pub enum CliError {
    /// DSL preflight or daemon-side validation failed.
    #[error("validation error: {0}")]
    Validation(String),

    /// Daemon returned `unknown_program`.
    #[error("not found: {0}")]
    NotFound(String),

    /// Daemon socket unreachable (ENOENT / ECONNREFUSED) — per spec §589 this
    /// is exit 4, distinct from a generic I/O failure.
    #[error("daemon not running (socket: {0})")]
    DaemonDown(String),

    /// Catch-all for I/O, malformed JSON on disk, bad arguments, etc.
    #[error("{0}")]
    Generic(String),
}

impl CliError {
    /// Map to the spec-mandated exit code.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Validation(_) => ExitCode::Validation,
            Self::NotFound(_) => ExitCode::NotFound,
            Self::DaemonDown(_) => ExitCode::DaemonDown,
            Self::Generic(_) => ExitCode::Generic,
        }
    }

    /// Build a `CliError` from a daemon-side `ErrorPayload`.
    #[must_use]
    pub fn from_payload(p: &ErrorPayload) -> Self {
        let text = format_payload(p);
        match p.code {
            ErrorCode::InvalidProgram => Self::Validation(text),
            ErrorCode::UnknownProgram => Self::NotFound(text),
            ErrorCode::BadStateTransition
            | ErrorCode::PermissionDenied
            | ErrorCode::CaptureFailed
            | ErrorCode::DaemonBusy
            | ErrorCode::NotResumable
            | ErrorCode::ResumePreconditionFailed
            | ErrorCode::Internal => Self::Generic(text),
        }
    }
}

fn format_payload(p: &ErrorPayload) -> String {
    use std::fmt::Write as _;
    let mut out = p.message.clone();
    if let Some(path) = &p.path {
        let _ = write!(out, " (at {path})");
    }
    if let (Some(line), Some(col)) = (p.line, p.column) {
        let _ = write!(out, " [line {line}, col {col}]");
    }
    if let Some(hint) = &p.hint {
        let _ = write!(out, " — did you mean `{hint}`?");
    }
    out
}

impl From<IpcError> for CliError {
    fn from(e: IpcError) -> Self {
        match e {
            IpcError::Io(ref io_err)
                if matches!(
                    io_err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                Self::DaemonDown(io_err.to_string())
            }
            other => Self::Generic(other.to_string()),
        }
    }
}

impl From<io::Error> for CliError {
    fn from(e: io::Error) -> Self {
        Self::Generic(e.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        Self::Generic(format!("json: {e}"))
    }
}

/// Convenience alias for command implementations.
pub type CliResult<T> = Result<T, CliError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_integers_match_spec() {
        assert_eq!(i32::from(ExitCode::Success), 0);
        assert_eq!(i32::from(ExitCode::Generic), 1);
        assert_eq!(i32::from(ExitCode::Validation), 2);
        assert_eq!(i32::from(ExitCode::NotFound), 3);
        assert_eq!(i32::from(ExitCode::DaemonDown), 4);
    }

    #[test]
    fn validation_error_maps_to_exit_2() {
        let e = CliError::Validation("bad".into());
        assert_eq!(e.exit_code(), ExitCode::Validation);
    }

    #[test]
    fn not_found_error_maps_to_exit_3() {
        let e = CliError::NotFound("abc".into());
        assert_eq!(e.exit_code(), ExitCode::NotFound);
    }

    #[test]
    fn daemon_down_maps_to_exit_4() {
        let e = CliError::DaemonDown("/tmp/x".into());
        assert_eq!(e.exit_code(), ExitCode::DaemonDown);
    }

    #[test]
    fn generic_falls_through_to_exit_1() {
        let e = CliError::Generic("oops".into());
        assert_eq!(e.exit_code(), ExitCode::Generic);
    }

    #[test]
    fn from_payload_invalid_program_is_validation() {
        let p = ErrorPayload::simple(ErrorCode::InvalidProgram, "unknown predicate");
        let e = CliError::from_payload(&p);
        assert!(matches!(e, CliError::Validation(_)));
    }

    #[test]
    fn from_payload_unknown_program_is_not_found() {
        let p = ErrorPayload::simple(ErrorCode::UnknownProgram, "no such id");
        let e = CliError::from_payload(&p);
        assert!(matches!(e, CliError::NotFound(_)));
    }

    #[test]
    fn from_payload_internal_is_generic() {
        let p = ErrorPayload::simple(ErrorCode::Internal, "boom");
        let e = CliError::from_payload(&p);
        assert!(matches!(e, CliError::Generic(_)));
    }

    #[test]
    fn from_payload_includes_path_line_col_hint() {
        let p = ErrorPayload {
            code: ErrorCode::InvalidProgram,
            message: "unknown predicate 'skp'".into(),
            path: Some("watches[0].when".into()),
            line: Some(12),
            column: Some(18),
            span_len: Some(3),
            hint: Some("skip".into()),
        };
        let e = CliError::from_payload(&p);
        let s = e.to_string();
        assert!(s.contains("watches[0].when"), "{s}");
        assert!(s.contains("line 12"), "{s}");
        assert!(s.contains("col 18"), "{s}");
        assert!(s.contains("skip"), "{s}");
    }

    #[test]
    fn ipc_not_found_maps_to_daemon_down() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "no socket");
        let e: CliError = IpcError::Io(io_err).into();
        assert!(matches!(e, CliError::DaemonDown(_)));
        assert_eq!(e.exit_code(), ExitCode::DaemonDown);
    }

    #[test]
    fn ipc_connection_refused_maps_to_daemon_down() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        let e: CliError = IpcError::Io(io_err).into();
        assert!(matches!(e, CliError::DaemonDown(_)));
    }

    #[test]
    fn ipc_other_io_maps_to_generic() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let e: CliError = IpcError::Io(io_err).into();
        assert!(matches!(e, CliError::Generic(_)));
    }
}
