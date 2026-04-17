//! Request envelope. Matches spec §IPC → Messages:
//!
//! ```jsonc
//! { "id": "<uuid>",
//!   "op": "submit"|"list"|"status"|"cancel"|"start"|"resume"|
//!          "logs"|"events"|"trace"|"health"|"gc"|"shutdown",
//!   "params": { /* op-specific */ } }
//! ```

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use vcli_core::ProgramId;

/// Correlation id on a single request/response pair. Not a `ProgramId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(Uuid);

impl RequestId {
    /// Generate a fresh id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for RequestId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Top-level request envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Client-generated correlation id.
    pub id: RequestId,
    /// Op + params, tagged via `op` on the wire.
    #[serde(flatten)]
    pub op: RequestOp,
}

/// All v0 ops. Tagged on the wire via `op`; params are variant payload.
/// Decision D removed `on_schedule`; Decision 2.3 removed `subprogram`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", content = "params", rename_all = "snake_case")]
pub enum RequestOp {
    /// Submit a new program. Body is the raw JSON program document (assets
    /// resolved by the daemon submit pipeline — Decision F4).
    Submit {
        /// Program document as JSON (daemon validates via `vcli-dsl`).
        program: serde_json::Value,
    },
    /// List programs, optionally filtered by state.
    List {
        /// Filter by `ProgramState`, e.g. `"running"`. `None` returns all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<String>,
    },
    /// Detailed status of one program.
    Status {
        /// Program id.
        program_id: ProgramId,
    },
    /// Cancel a running program.
    Cancel {
        /// Program id.
        program_id: ProgramId,
    },
    /// Fire a manual trigger.
    Start {
        /// Program id.
        program_id: ProgramId,
    },
    /// Resume a program that ended with `failed(daemon_restart)`.
    Resume {
        /// Program id.
        program_id: ProgramId,
        /// Ignore `body_cursor` and restart from step 0.
        #[serde(default)]
        from_start: bool,
    },
    /// Stream program-scoped events (one program only).
    Logs {
        /// Program id.
        program_id: ProgramId,
        /// If true, keep stream open and push future events.
        #[serde(default)]
        follow: bool,
    },
    /// Stream all events (firehose).
    Events {
        /// If true, keep stream open and push future events.
        #[serde(default)]
        follow: bool,
    },
    /// Dump trace buffer for a program.
    Trace {
        /// Program id.
        program_id: ProgramId,
    },
    /// Return daemon version, uptime, cache sizes, tick histogram.
    Health,
    /// Trigger asset garbage collection.
    Gc,
    /// Request graceful daemon shutdown.
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn submit_roundtrips_with_program_payload() {
        let req = Request {
            id: RequestId::new(),
            op: RequestOp::Submit {
                program: serde_json::json!({ "version": "0.1", "name": "x" }),
            },
        };
        let j = serde_json::to_string(&req).unwrap();
        assert!(j.contains(r#""op":"submit""#), "{j}");
        let back: Request = serde_json::from_str(&j).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn list_with_no_filter_omits_params_field() {
        let req = Request {
            id: RequestId::new(),
            op: RequestOp::List { state: None },
        };
        let j = serde_json::to_string(&req).unwrap();
        assert!(j.contains(r#""op":"list""#), "{j}");
        let back: Request = serde_json::from_str(&j).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn cancel_requires_program_id() {
        let req = Request {
            id: RequestId::new(),
            op: RequestOp::Cancel { program_id: pid() },
        };
        let j = serde_json::to_string(&req).unwrap();
        assert!(j.contains(r#""op":"cancel""#), "{j}");
        let back: Request = serde_json::from_str(&j).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn resume_from_start_flag_roundtrips() {
        let req = Request {
            id: RequestId::new(),
            op: RequestOp::Resume {
                program_id: pid(),
                from_start: true,
            },
        };
        let j = serde_json::to_string(&req).unwrap();
        assert!(j.contains(r#""from_start":true"#), "{j}");
        let back: Request = serde_json::from_str(&j).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn health_gc_shutdown_have_no_params() {
        for op in [RequestOp::Health, RequestOp::Gc, RequestOp::Shutdown] {
            let req = Request {
                id: RequestId::new(),
                op: op.clone(),
            };
            let j = serde_json::to_string(&req).unwrap();
            let back: Request = serde_json::from_str(&j).unwrap();
            assert_eq!(back, req);
        }
    }

    #[test]
    fn unknown_op_rejected() {
        let bogus = r#"{"id":"12345678-1234-4567-8910-111213141516","op":"teleport"}"#;
        let err = serde_json::from_str::<Request>(bogus);
        assert!(err.is_err());
    }
}
