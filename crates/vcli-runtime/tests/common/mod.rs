//! Shared test helpers for vcli-runtime scenario tests.

#![allow(dead_code)] // each scenario uses a different subset

use std::collections::BTreeMap;

pub use crossbeam_channel::{unbounded, Receiver, Sender};
pub use vcli_core::state::ProgramState;
pub use vcli_core::{Event, ProgramId};
pub use vcli_runtime::{Scheduler, SchedulerCommand, SchedulerConfig};

pub mod mock_capture;
pub mod mock_input;

pub use mock_capture::ScriptedCapture;
pub use mock_input::RecordingInputSink;

/// Extract the `type` tag from a serialized event, for pattern-matching in assertions.
#[must_use]
pub fn event_type(e: &Event) -> String {
    serde_json::to_value(e).unwrap()["type"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// Drain a channel into a Vec (non-blocking).
#[must_use]
pub fn drain_events(rx: &Receiver<Event>) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

/// Build an empty predicate map (convenience for scenarios).
#[must_use]
pub fn empty_predicates() -> BTreeMap<String, vcli_core::Predicate> {
    BTreeMap::new()
}
