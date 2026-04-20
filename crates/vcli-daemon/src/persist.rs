//! Event pipeline. The scheduler pushes `vcli_core::Event`s into
//! `crossbeam_channel::Receiver<Event>`. A dedicated tokio task drains that
//! receiver, calls `store.append_event` for every program-scoped variant
//! (`DaemonStarted` / `DaemonStopped` / `DaemonPressure` / `StreamDropped`
//! etc. are broadcast-only — no program id, no DB row), then forwards the
//! event into the `broadcast::Sender`.
//!
//! Ordering invariant (Decision 1.7): an event is in the DB before any
//! subscriber can see it.

use std::sync::{Arc, Mutex};

use crossbeam_channel::Receiver;
use tokio::sync::broadcast;
use tracing::{error, trace};
use vcli_core::{Event, EventData, ProgramId};
use vcli_store::Store;

/// Drain `event_rx` on a background tokio task. Returns a `JoinHandle` so the
/// run loop can await shutdown.
///
/// # Panics
/// The background task panics if the store mutex is poisoned.
pub fn spawn_event_pump(
    store: Arc<Mutex<Store>>,
    event_rx: Receiver<Event>,
    broadcast_tx: broadcast::Sender<Event>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while let Ok(ev) = event_rx.recv() {
            if let Some(pid) = program_id_of(&ev.data) {
                let mut guard = store.lock().expect("store mutex poisoned");
                if let Err(e) = guard.append_event(pid, &ev) {
                    error!(error = %e, "failed to append event");
                }
            }
            let _ = broadcast_tx.send(ev);
            trace!("event pump iteration");
        }
    })
}

/// Extract the program id from an event payload, if any. Returns `None` for
/// daemon-scoped events that are broadcast-only.
#[must_use]
pub fn program_id_of(d: &EventData) -> Option<ProgramId> {
    match d {
        EventData::ProgramSubmitted { program_id, .. }
        | EventData::ProgramStateChanged { program_id, .. }
        | EventData::ProgramCompleted { program_id, .. }
        | EventData::ProgramFailed { program_id, .. }
        | EventData::ProgramResumed { program_id, .. }
        | EventData::WatchFired { program_id, .. }
        | EventData::ActionDispatched { program_id, .. }
        | EventData::ActionDeferred { program_id, .. } => Some(*program_id),
        EventData::DaemonStarted { .. }
        | EventData::DaemonStopped
        | EventData::DaemonPressure { .. }
        | EventData::StreamDropped { .. }
        | EventData::TickFrameSkipped { .. }
        | EventData::CapturePermissionMissing { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use tempfile::tempdir;
    use vcli_core::EventData;
    use vcli_store::{NewProgram, Store};

    #[tokio::test]
    async fn program_event_is_appended_before_broadcast() {
        let d = tempdir().unwrap();
        let (mut store, _) = Store::open(d.path()).unwrap();
        let pid = ProgramId::new();
        store
            .insert_program(&NewProgram {
                id: pid,
                name: "x",
                source_json: "{}",
                state: vcli_core::ProgramState::Pending,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
        let store = Arc::new(Mutex::new(store));
        let (sched_tx, sched_rx) = unbounded::<Event>();
        let (bcast_tx, mut bcast_rx) = broadcast::channel::<Event>(16);

        let pump = spawn_event_pump(store.clone(), sched_rx, bcast_tx);

        let ev = Event {
            at: 7,
            data: EventData::ProgramCompleted {
                program_id: pid,
                emit: None,
            },
        };
        sched_tx.send(ev.clone()).unwrap();
        let got = bcast_rx.recv().await.unwrap();
        assert_eq!(got, ev);
        let rows = store.lock().unwrap().stream_events(0, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].type_tag, "program.completed");

        drop(sched_tx);
        pump.await.unwrap();
    }

    #[test]
    fn daemon_started_has_no_program_id() {
        let d = EventData::DaemonStarted {
            version: "0.0.1".into(),
        };
        assert!(program_id_of(&d).is_none());
    }

    #[test]
    fn state_changed_has_program_id() {
        let p = ProgramId::new();
        let d = EventData::ProgramStateChanged {
            program_id: p,
            from: vcli_core::ProgramState::Waiting,
            to: vcli_core::ProgramState::Running,
            reason: "trigger".into(),
        };
        assert_eq!(program_id_of(&d), Some(p));
    }
}
