//! Program lifecycle state. See spec §Runtime → Program state machine.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Lifecycle state of a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramState {
    /// Submitted but daemon not yet ready / program not yet loaded into scheduler.
    Pending,
    /// Loaded; waiting for trigger to fire.
    Waiting,
    /// Trigger fired; scheduler is advancing body + watches.
    Running,
    /// Reserved — no v0 transition enters this.
    Blocked,
    /// Body complete (non-empty body) or last watch removed (pure-watches programs).
    Completed,
    /// Body error, assert failure, timeout, `capture_failed`, or `daemon_restart`.
    Failed,
    /// Explicit `vcli cancel`.
    Cancelled,
}

impl ProgramState {
    /// Whether this is a terminal state (no further transitions).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Whether this state represents active execution.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, Self::Waiting | Self::Running | Self::Blocked)
    }

    /// Canonical string form (same as serde `snake_case`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for ProgramState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error parsing a `ProgramState` from a string.
#[derive(Debug, Error)]
#[error("unknown program state: {0}")]
pub struct ProgramStateParseError(pub String);

impl FromStr for ProgramState {
    type Err = ProgramStateParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "pending" => Self::Pending,
            "waiting" => Self::Waiting,
            "running" => Self::Running,
            "blocked" => Self::Blocked,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            other => return Err(ProgramStateParseError(other.to_string())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_classification() {
        assert!(ProgramState::Completed.is_terminal());
        assert!(ProgramState::Failed.is_terminal());
        assert!(ProgramState::Cancelled.is_terminal());
        assert!(!ProgramState::Running.is_terminal());
        assert!(!ProgramState::Waiting.is_terminal());
    }

    #[test]
    fn active_classification() {
        assert!(ProgramState::Waiting.is_active());
        assert!(ProgramState::Running.is_active());
        assert!(ProgramState::Blocked.is_active());
        assert!(!ProgramState::Pending.is_active());
        assert!(!ProgramState::Completed.is_active());
    }

    #[test]
    fn display_parse_roundtrip() {
        for s in [
            ProgramState::Pending,
            ProgramState::Waiting,
            ProgramState::Running,
            ProgramState::Blocked,
            ProgramState::Completed,
            ProgramState::Failed,
            ProgramState::Cancelled,
        ] {
            let back: ProgramState = s.to_string().parse().unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn serde_snake_case_matches_as_str() {
        let j = serde_json::to_string(&ProgramState::Running).unwrap();
        assert_eq!(j, r#""running""#);
    }

    #[test]
    fn parse_rejects_unknown() {
        let r: Result<ProgramState, _> = "invalid".parse();
        assert!(r.is_err());
    }
}
