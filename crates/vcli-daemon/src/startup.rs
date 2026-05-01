//! Startup tasks: emit events for orphan recovery, reload waiting programs,
//! publish `daemon.started`. Runs once on every boot; idempotent given the
//! `RecoveredProgram` set from `Store::open` is fresh each call.

use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use tracing::info;

use vcli_core::{Event, EventData, ProgramState};
use vcli_store::{RecoveredProgram, Store};

/// Emit one `program.state_changed` + one `program.failed` for every program
/// that `Store::open` rewrote from `running` → `failed(daemon_restart)`.
/// The events are sent on `sched_event_tx` (scheduler → pump side), so the
/// persistence pipeline sees them first.
pub fn emit_recovery_events(recovered: &[RecoveredProgram], sched_event_tx: &Sender<Event>) {
    let now = vcli_core::clock::now_unix_ms();
    for r in recovered {
        let ev1 = Event {
            at: now,
            data: EventData::ProgramStateChanged {
                program_id: r.id,
                from: ProgramState::Running,
                to: ProgramState::Failed,
                reason: "daemon_restart".into(),
            },
        };
        let ev2 = Event {
            at: now,
            data: EventData::ProgramFailed {
                program_id: r.id,
                reason: "daemon restarted during execution".into(),
                step: Some(format!("body[{}]", r.body_cursor)),
                emit: None,
            },
        };
        let _ = sched_event_tx.send(ev1);
        let _ = sched_event_tx.send(ev2);
    }
    info!(count = recovered.len(), "emitted recovery events");
}

/// Emit a `daemon.started` event.
pub fn emit_daemon_started(sched_event_tx: &Sender<Event>) {
    let ev = Event {
        at: vcli_core::clock::now_unix_ms(),
        data: EventData::DaemonStarted {
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    let _ = sched_event_tx.send(ev);
}

/// Reload programs currently in `waiting` state: enqueue a scheduler command
/// per program so the scheduler re-installs its trigger. This is the second
/// half of spec §Restart semantics step 5.
///
/// # Panics
/// Panics if the store mutex is poisoned.
pub fn reload_waiting_programs(
    store: &Arc<Mutex<Store>>,
    cmd_tx: &crossbeam_channel::Sender<crate::bridge::SchedulerCommand>,
) -> usize {
    let now_ms = vcli_core::clock::now_unix_ms();
    let reloaded = {
        let mut s = store.lock().unwrap();
        let rows = s
            .list_programs(Some(ProgramState::Waiting))
            .unwrap_or_default();
        rows.into_iter()
            .filter_map(|row| {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&row.source_json) else {
                    return None;
                };
                let Ok(mut program) = vcli_dsl::validate_value(&value).map(|v| v.program) else {
                    return None;
                };
                let Ok(materialized) = crate::assets::materialize_template_assets(
                    &mut s,
                    row.id,
                    &mut program,
                    None,
                    now_ms,
                ) else {
                    return None;
                };
                Some((row.id, program, materialized.bytes))
            })
            .collect::<Vec<_>>()
    };
    let mut sent = 0;
    for (program_id, program, assets) in reloaded {
        if cmd_tx
            .send(crate::bridge::SchedulerCommand::SubmitValidated {
                program_id,
                program,
                assets,
            })
            .is_ok()
        {
            sent += 1;
        }
    }
    info!(count = sent, "reloaded waiting programs");
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use tempfile::tempdir;

    #[test]
    fn emits_two_events_per_recovered_program() {
        let (tx, rx) = unbounded::<Event>();
        let recovered = vec![RecoveredProgram {
            id: vcli_core::ProgramId::new(),
            body_cursor: 2,
        }];
        emit_recovery_events(&recovered, &tx);
        let first = rx.recv().unwrap();
        let second = rx.recv().unwrap();
        assert!(matches!(first.data, EventData::ProgramStateChanged { .. }));
        assert!(matches!(second.data, EventData::ProgramFailed { .. }));
    }

    #[test]
    fn emit_daemon_started_sends_one_event() {
        let (tx, rx) = unbounded::<Event>();
        emit_daemon_started(&tx);
        assert!(matches!(
            rx.recv().unwrap().data,
            EventData::DaemonStarted { .. }
        ));
    }

    #[test]
    fn reload_waiting_does_nothing_when_none_exist() {
        let d = tempdir().unwrap();
        let (store, _) = Store::open(d.path()).unwrap();
        let store = Arc::new(Mutex::new(store));
        let (cmd_tx, _cmd_rx) = unbounded::<crate::bridge::SchedulerCommand>();
        let n = reload_waiting_programs(&store, &cmd_tx);
        assert_eq!(n, 0);
    }

    #[test]
    fn reload_waiting_sends_template_assets() {
        let d = tempdir().unwrap();
        let (mut store, _) = Store::open(d.path()).unwrap();
        let pid = vcli_core::ProgramId::new();
        let stored = store.put_asset(b"PNG-BYTES", Some("png"), 0).unwrap();
        let hash = stored.hash.hex().to_string();
        let source_json = serde_json::json!({
            "version": "0.1",
            "name": "waiting-template",
            "trigger": { "kind": "on_submit" },
            "predicates": {
                "skip": {
                    "kind": "template",
                    "image": format!("sha256:{hash}"),
                    "confidence": 0.9,
                    "region": {
                        "kind": "absolute",
                        "box": { "x": 0, "y": 0, "w": 10, "h": 10 }
                    }
                }
            },
            "watches": [],
            "body": [],
        })
        .to_string();
        store
            .insert_program(&vcli_store::NewProgram {
                id: pid,
                name: "waiting-template",
                source_json: &source_json,
                state: ProgramState::Waiting,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
        let store = Arc::new(Mutex::new(store));
        let (cmd_tx, cmd_rx) = unbounded::<crate::bridge::SchedulerCommand>();

        let n = reload_waiting_programs(&store, &cmd_tx);

        assert_eq!(n, 1);
        match cmd_rx.recv().unwrap() {
            crate::bridge::SchedulerCommand::SubmitValidated {
                program_id, assets, ..
            } => {
                assert_eq!(program_id, pid);
                assert_eq!(
                    assets.get(&hash).map(Vec::as_slice),
                    Some(&b"PNG-BYTES"[..])
                );
            }
            other => panic!("wrong cmd: {other:?}"),
        }
    }
}
