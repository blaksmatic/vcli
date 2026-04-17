//! Append-only durable event log (spec §Persistence + Decision 1.7).

use vcli_core::clock::UnixMs;
use vcli_core::events::{Event, EventData};
use vcli_core::ids::ProgramId;

use crate::error::StoreResult;
use crate::store::Store;

/// A row read back from the `events` table.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    /// Autoincrement row id. Also serves as a stream cursor.
    pub id: i64,
    /// Program this event refers to.
    pub program_id: ProgramId,
    /// Wire type tag (e.g. `"program.completed"`).
    pub type_tag: String,
    /// Raw JSON payload (the serialized `Event`).
    pub data_json: String,
    /// Wall-clock timestamp.
    pub at: UnixMs,
}

impl Store {
    /// Append a persisted event row. `program_id` must refer to an existing
    /// program (FK enforced).
    ///
    /// # Errors
    /// Surfaces `SQLite` + serde errors.
    pub fn append_event(&mut self, program_id: ProgramId, ev: &Event) -> StoreResult<i64> {
        let type_tag = event_type_tag(&ev.data).to_string();
        let data_json = serde_json::to_string(ev)?;
        self.conn_mut().execute(
            "INSERT INTO events (program_id, type, data_json, at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![program_id.to_string(), type_tag, data_json, ev.at],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Stream events with `id > since`, ordered by id ascending.
    /// Pass `since = 0` to read everything. `limit` caps the returned rows.
    ///
    /// # Errors
    /// Surfaces `SQLite` errors.
    ///
    /// # Panics
    /// Panics if a stored program id is not a valid UUID (should never happen for
    /// data written by this crate).
    pub fn stream_events(&self, since: i64, limit: u32) -> StoreResult<Vec<StoredEvent>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, program_id, type, data_json, at
             FROM events
             WHERE id > ?1
             ORDER BY id ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![since, i64::from(limit)], |r| {
            let id: i64 = r.get(0)?;
            let pid_s: String = r.get(1)?;
            let type_tag: String = r.get(2)?;
            let data_json: String = r.get(3)?;
            let at: i64 = r.get(4)?;
            Ok(StoredEvent {
                id,
                program_id: pid_s.parse().unwrap(),
                type_tag,
                data_json,
                at,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn event_type_tag(d: &EventData) -> &'static str {
    match d {
        EventData::ProgramSubmitted { .. } => "program.submitted",
        EventData::ProgramStateChanged { .. } => "program.state_changed",
        EventData::ProgramCompleted { .. } => "program.completed",
        EventData::ProgramFailed { .. } => "program.failed",
        EventData::ProgramResumed { .. } => "program.resumed",
        EventData::WatchFired { .. } => "watch.fired",
        EventData::ActionDispatched { .. } => "action.dispatched",
        EventData::ActionDeferred { .. } => "action.deferred",
        EventData::TickFrameSkipped { .. } => "tick.frame_skipped",
        EventData::DaemonPressure { .. } => "daemon.pressure",
        EventData::StreamDropped { .. } => "stream.dropped",
        EventData::CapturePermissionMissing { .. } => "capture.permission_missing",
        EventData::DaemonStarted { .. } => "daemon.started",
        EventData::DaemonStopped => "daemon.stopped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewProgram;
    use tempfile::tempdir;
    use vcli_core::state::ProgramState;

    fn seed(s: &mut Store) -> ProgramId {
        let id = ProgramId::new();
        s.insert_program(&NewProgram {
            id,
            name: "x",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
        id
    }

    #[test]
    fn append_event_returns_monotonic_ids() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = seed(&mut s);
        let ev = Event {
            at: 10,
            data: EventData::ProgramCompleted {
                program_id: id,
                emit: None,
            },
        };
        let a = s.append_event(id, &ev).unwrap();
        let b = s.append_event(id, &ev).unwrap();
        assert!(b > a);
    }

    #[test]
    fn stream_events_returns_since_cursor() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = seed(&mut s);
        for i in 0..5 {
            s.append_event(
                id,
                &Event {
                    at: i,
                    data: EventData::ProgramStateChanged {
                        program_id: id,
                        from: ProgramState::Waiting,
                        to: ProgramState::Running,
                        reason: format!("t{i}"),
                    },
                },
            )
            .unwrap();
        }
        let first = s.stream_events(0, 10).unwrap();
        assert_eq!(first.len(), 5);
        let tail = s.stream_events(first[2].id, 10).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].id, first[3].id);
    }

    #[test]
    fn stream_events_respects_limit() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = seed(&mut s);
        for _ in 0..10 {
            s.append_event(
                id,
                &Event {
                    at: 0,
                    data: EventData::ProgramCompleted {
                        program_id: id,
                        emit: None,
                    },
                },
            )
            .unwrap();
        }
        let rows = s.stream_events(0, 3).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn cascade_delete_removes_events() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = seed(&mut s);
        s.append_event(
            id,
            &Event {
                at: 0,
                data: EventData::ProgramCompleted {
                    program_id: id,
                    emit: None,
                },
            },
        )
        .unwrap();
        s.conn_mut()
            .execute("DELETE FROM programs WHERE id = ?1", [id.to_string()])
            .unwrap();
        let rows = s.stream_events(0, 10).unwrap();
        assert!(rows.is_empty());
    }
}
