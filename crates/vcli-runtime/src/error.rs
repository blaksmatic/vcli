//! Typed errors with a stable `code()` string suitable for IPC wire values.

use thiserror::Error;

/// Stable error-code string embedded in `program.failed.reason` and returned
/// to callers via IPC. Matches spec §555 (CLI error codes) and §713 (internal
/// surfaces).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// `capture.grab_screen()` or `grab_window()` failed.
    CaptureFailed,
    /// `InputSink` returned an error or the kill switch fired.
    InputFailed,
    /// A predicate evaluator raised an error (unknown name, asset missing, decode).
    PerceptionFailed,
    /// `wait_for` ran out of budget.
    WaitForTimeout,
    /// An `assert` predicate was not truthy.
    AssertFailed,
    /// A program's `timeout_ms` elapsed before completion.
    ProgramTimeout,
    /// Input postcondition never observed (`novelty_timeout` reached).
    NoveltyTimeout,
    /// An expression failed to resolve (`$pred.match.center` on a non-match).
    ExpressionUnresolved,
    /// Daemon restart marker (transitioned by `Scheduler::on_startup`).
    DaemonRestart,
    /// Catch-all for programmer errors (unreachable panics demoted to errors).
    Internal,
}

impl ErrorCode {
    /// Canonical wire-value string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CaptureFailed => "capture_failed",
            Self::InputFailed => "input_failed",
            Self::PerceptionFailed => "perception_failed",
            Self::WaitForTimeout => "wait_for_timeout",
            Self::AssertFailed => "assert_failed",
            Self::ProgramTimeout => "program_timeout",
            Self::NoveltyTimeout => "novelty_timeout",
            Self::ExpressionUnresolved => "expression_unresolved",
            Self::DaemonRestart => "daemon_restart",
            Self::Internal => "internal",
        }
    }
}

/// Runtime-layer error. Carries a [`ErrorCode`] plus a human reason string.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Capture backend failure.
    #[error("capture_failed: {0}")]
    Capture(String),
    /// Input dispatch failure.
    #[error("input_failed: {0}")]
    Input(String),
    /// Perception evaluator failure.
    #[error("perception_failed: {0}")]
    Perception(String),
    /// `wait_for` timed out.
    #[error("wait_for_timeout: predicate={predicate} after {waited_ms}ms")]
    WaitForTimeout {
        /// Predicate name.
        predicate: String,
        /// Budget actually consumed.
        waited_ms: u32,
    },
    /// `assert` predicate not truthy.
    #[error("assert_failed: predicate={predicate}")]
    AssertFailed {
        /// Predicate name.
        predicate: String,
    },
    /// Program-level timeout fired.
    #[error("program_timeout: after {elapsed_ms}ms")]
    ProgramTimeout {
        /// How long we ran before tripping.
        elapsed_ms: u32,
    },
    /// Postcondition not observed.
    #[error("novelty_timeout: no postcondition change within {timeout_ms}ms")]
    NoveltyTimeout {
        /// Budget configured on the Step.
        timeout_ms: u32,
    },
    /// Expression unresolved.
    #[error("expression_unresolved: {0}")]
    ExpressionUnresolved(String),
    /// Internal invariant violation.
    #[error("internal: {0}")]
    Internal(String),
}

impl RuntimeError {
    /// Wire-stable code. Propagated to `program.failed.reason`.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Capture(_) => ErrorCode::CaptureFailed,
            Self::Input(_) => ErrorCode::InputFailed,
            Self::Perception(_) => ErrorCode::PerceptionFailed,
            Self::WaitForTimeout { .. } => ErrorCode::WaitForTimeout,
            Self::AssertFailed { .. } => ErrorCode::AssertFailed,
            Self::ProgramTimeout { .. } => ErrorCode::ProgramTimeout,
            Self::NoveltyTimeout { .. } => ErrorCode::NoveltyTimeout,
            Self::ExpressionUnresolved(_) => ErrorCode::ExpressionUnresolved,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_strings_are_spec_stable() {
        assert_eq!(ErrorCode::CaptureFailed.as_str(), "capture_failed");
        assert_eq!(ErrorCode::WaitForTimeout.as_str(), "wait_for_timeout");
        assert_eq!(ErrorCode::NoveltyTimeout.as_str(), "novelty_timeout");
    }

    #[test]
    fn runtime_error_maps_to_code() {
        let e = RuntimeError::WaitForTimeout {
            predicate: "p".into(),
            waited_ms: 1000,
        };
        assert_eq!(e.code(), ErrorCode::WaitForTimeout);
        assert_eq!(e.code().as_str(), "wait_for_timeout");
    }
}
