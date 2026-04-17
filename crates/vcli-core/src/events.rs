//! Daemon-wide event taxonomy. Matches spec §IPC → Events (v0).
//!
//! Every variant is tagged with the wire type string via `#[serde(rename)]`
//! so the JSON emitted over IPC matches the spec exactly (e.g. `"program.state_changed"`
//! rather than the Rust `snake_case` `"program_state_changed"`).

use serde::{Deserialize, Serialize};

use crate::clock::UnixMs;
use crate::ids::ProgramId;
use crate::state::ProgramState;

/// Envelope pushed on streaming IPC channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Wall-clock timestamp when the event was produced.
    pub at: UnixMs,
    /// Typed payload.
    #[serde(flatten)]
    pub data: EventData,
}

/// Event payloads. Tagged on the wire via `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventData {
    /// A program was accepted by the daemon.
    #[serde(rename = "program.submitted")]
    ProgramSubmitted {
        /// Program id.
        program_id: ProgramId,
        /// Program `name` field.
        name: String,
    },
    /// Program transitioned between lifecycle states.
    #[serde(rename = "program.state_changed")]
    ProgramStateChanged {
        /// Program id.
        program_id: ProgramId,
        /// Prior state.
        from: ProgramState,
        /// New state.
        to: ProgramState,
        /// Human-readable reason.
        reason: String,
    },
    /// Program reached `completed`.
    #[serde(rename = "program.completed")]
    ProgramCompleted {
        /// Program id.
        program_id: ProgramId,
        /// Custom emit name from `on_complete.emit`, if set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emit: Option<String>,
    },
    /// Program reached `failed`.
    #[serde(rename = "program.failed")]
    ProgramFailed {
        /// Program id.
        program_id: ProgramId,
        /// Human-readable reason.
        reason: String,
        /// Step path (e.g. "body[2]") where the failure originated.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<String>,
        /// Custom emit name from `on_fail.emit`, if set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emit: Option<String>,
    },
    /// Program was resumed from a previous `daemon_restart` failure.
    #[serde(rename = "program.resumed")]
    ProgramResumed {
        /// Program id.
        program_id: ProgramId,
        /// Step index execution resumed at (0 = --from-start).
        from_step: u32,
    },
    /// A watch fired.
    #[serde(rename = "watch.fired")]
    WatchFired {
        /// Program id.
        program_id: ProgramId,
        /// Index into the program's `watches` array.
        watch_index: u32,
        /// Predicate name or `"inline"` for anonymous predicates.
        predicate: String,
    },
    /// An input action was dispatched.
    #[serde(rename = "action.dispatched")]
    ActionDispatched {
        /// Program id.
        program_id: ProgramId,
        /// Serialized action step, for tracing.
        step: serde_json::Value,
        /// Resolved target point if applicable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<crate::geom::Point>,
    },
    /// An action was dropped due to arbiter conflict.
    #[serde(rename = "action.deferred")]
    ActionDeferred {
        /// Program id.
        program_id: ProgramId,
        /// Serialized action step.
        step: serde_json::Value,
        /// Reason for deferral (e.g. `"conflict_with": "<program_id>"`).
        reason: serde_json::Value,
    },
    /// A tick was skipped (e.g. capture overrun).
    #[serde(rename = "tick.frame_skipped")]
    TickFrameSkipped {
        /// Reason tag.
        reason: String,
    },
    /// Scheduler is running sustained over budget (Decision 4.1).
    #[serde(rename = "daemon.pressure")]
    DaemonPressure {
        /// Target tick budget in ms (usually 90).
        tick_budget_ms: u32,
    },
    /// Stream buffer overflow — clients missed events (Decision 1.7).
    #[serde(rename = "stream.dropped")]
    StreamDropped {
        /// Number of dropped events.
        count: u32,
        /// Timestamp of first dropped event.
        since: UnixMs,
    },
    /// Daemon is missing a required permission.
    #[serde(rename = "capture.permission_missing")]
    CapturePermissionMissing {
        /// Backend identifier (e.g. `"screencapturekit"`).
        backend: String,
    },
    /// Daemon started.
    #[serde(rename = "daemon.started")]
    DaemonStarted {
        /// Daemon version string.
        version: String,
    },
    /// Daemon stopped.
    #[serde(rename = "daemon.stopped")]
    DaemonStopped,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_program_id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn program_submitted_serializes_with_typed_tag() {
        let e = Event {
            at: 1_700_000_000_000,
            data: EventData::ProgramSubmitted {
                program_id: sample_program_id(),
                name: "yt".into(),
            },
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains(r#""type":"program.submitted""#), "got {j}");
        assert!(j.contains(r#""name":"yt""#));
        let back: Event = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn program_completed_omits_emit_when_none() {
        let e = Event {
            at: 0,
            data: EventData::ProgramCompleted {
                program_id: sample_program_id(),
                emit: None,
            },
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(!j.contains("\"emit\""), "got {j}");
    }

    #[test]
    fn program_failed_carries_optional_step_and_emit() {
        let e = Event {
            at: 0,
            data: EventData::ProgramFailed {
                program_id: sample_program_id(),
                reason: "wait_for timed out".into(),
                step: Some("body[2]".into()),
                emit: Some("buy_failed".into()),
            },
        };
        let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn watch_fired_roundtrip() {
        let e = Event {
            at: 1,
            data: EventData::WatchFired {
                program_id: sample_program_id(),
                watch_index: 0,
                predicate: "skip_visible".into(),
            },
        };
        let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn stream_dropped_roundtrip() {
        let e = Event {
            at: 2,
            data: EventData::StreamDropped {
                count: 5,
                since: 100,
            },
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains(r#""type":"stream.dropped""#));
        let back: Event = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn daemon_pressure_and_frame_skipped_roundtrip() {
        for e in [
            Event {
                at: 0,
                data: EventData::DaemonPressure { tick_budget_ms: 90 },
            },
            Event {
                at: 0,
                data: EventData::TickFrameSkipped {
                    reason: "capture_overrun".into(),
                },
            },
        ] {
            let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
            assert_eq!(back, e);
        }
    }

    #[test]
    fn daemon_started_stopped_roundtrip() {
        for e in [
            Event {
                at: 1,
                data: EventData::DaemonStarted {
                    version: "0.0.1".into(),
                },
            },
            Event {
                at: 2,
                data: EventData::DaemonStopped,
            },
        ] {
            let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
            assert_eq!(back, e);
        }
    }
}
