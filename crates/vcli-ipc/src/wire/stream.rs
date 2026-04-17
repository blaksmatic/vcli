//! Streaming frames pushed by the server on `logs` / `events` / `trace` ops.
//! Matches spec §IPC → Messages (Event form) and Decision 1.7 (`stream.dropped`).

use serde::{Deserialize, Serialize};

use vcli_core::Event;

use super::request::RequestId;

/// Which logical stream a frame belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// Program-scoped or global event stream.
    Events,
    /// Per-program trace dump.
    Trace,
}

/// One frame on an open stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamFrame {
    /// Correlation id of the originating request.
    pub id: RequestId,
    /// Which stream this frame belongs to.
    pub stream: StreamKind,
    /// When true, no further frames; client should stop reading.
    #[serde(default, skip_serializing_if = "is_false")]
    pub end: bool,
    /// Event payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
    /// Raw trace record JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<serde_json::Value>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl StreamFrame {
    /// Convenience: build an event frame.
    #[must_use]
    pub fn event(id: RequestId, event: Event) -> Self {
        Self { id, stream: StreamKind::Events, end: false, event: Some(event), trace: None }
    }

    /// Convenience: build a trace record frame.
    #[must_use]
    pub fn trace(id: RequestId, record: serde_json::Value) -> Self {
        Self { id, stream: StreamKind::Trace, end: false, event: None, trace: Some(record) }
    }

    /// Terminal frame signalling end of stream.
    #[must_use]
    pub fn end_of_stream(id: RequestId, kind: StreamKind) -> Self {
        Self { id, stream: kind, end: true, event: None, trace: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::EventData;

    fn rid() -> RequestId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn event_frame_serializes_with_kind_events() {
        let f = StreamFrame::event(
            rid(),
            Event {
                at: 1,
                data: EventData::DaemonStarted { version: "0.0.1".into() },
            },
        );
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains(r#""stream":"events""#), "{j}");
        assert!(j.contains(r#""type":"daemon.started""#), "{j}");
        assert!(!j.contains(r#""end":"#), "{j}");
        let back: StreamFrame = serde_json::from_str(&j).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn trace_frame_carries_opaque_json() {
        let f = StreamFrame::trace(rid(), serde_json::json!({ "tick": 12 }));
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains(r#""stream":"trace""#), "{j}");
        let back: StreamFrame = serde_json::from_str(&j).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn end_of_stream_flag_is_emitted_and_parsed() {
        let f = StreamFrame::end_of_stream(rid(), StreamKind::Events);
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains(r#""end":true"#), "{j}");
        let back: StreamFrame = serde_json::from_str(&j).unwrap();
        assert!(back.end);
        assert!(back.event.is_none());
        assert!(back.trace.is_none());
    }
}
