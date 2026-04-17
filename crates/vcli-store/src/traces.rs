//! Durable side of the trace ring. The in-memory ring lives in `vcli-runtime`;
//! this module persists `TraceRecord`s in batch at task boundaries or shutdown.

use serde::{Deserialize, Serialize};

use vcli_core::clock::UnixMs;
use vcli_core::ids::ProgramId;

use crate::error::StoreResult;
use crate::store::Store;

/// Classification of a trace record. Matches spec §Trace buffer (in-memory).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceKind {
    /// A predicate evaluation produced a fresh result.
    PredicateEval,
    /// A program changed state.
    StateChange,
    /// An action was dispatched.
    ActionDispatched,
    /// An action was deferred due to arbiter conflict.
    ActionDeferred,
    /// A watch became eligible and fired.
    WatchFired,
    /// A tick was skipped (e.g. capture overrun).
    TickSkipped,
}

impl TraceKind {
    /// Canonical wire form used in the DB `kind` column.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PredicateEval => "predicate_eval",
            Self::StateChange => "state_change",
            Self::ActionDispatched => "action_dispatched",
            Self::ActionDeferred => "action_deferred",
            Self::WatchFired => "watch_fired",
            Self::TickSkipped => "tick_skipped",
        }
    }
}

/// A single trace record ready to persist or to place on the in-memory ring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceRecord {
    /// Monotonic tick index.
    pub tick: u64,
    /// Wall-clock timestamp.
    pub at: UnixMs,
    /// Owning program, if any.
    pub program_id: Option<ProgramId>,
    /// Record kind.
    pub kind: TraceKind,
    /// Freeform JSON payload.
    pub payload: serde_json::Value,
}

impl Store {
    /// Persist a batch of trace records. Single transaction so either the
    /// whole batch lands or none. Caller is responsible for choosing when to
    /// flush (spec: "at task boundaries or on shutdown").
    ///
    /// # Errors
    /// Surfaces `SQLite` + serde errors.
    pub fn flush_traces(&mut self, records: &[TraceRecord]) -> StoreResult<()> {
        if records.is_empty() {
            return Ok(());
        }
        let tx = self.conn_mut().transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO traces (program_id, tick, at, kind, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for r in records {
                let pid: Option<String> = r.program_id.map(|p| p.to_string());
                stmt.execute(rusqlite::params![
                    pid,
                    i64::try_from(r.tick).unwrap_or(i64::MAX),
                    r.at,
                    r.kind.as_str(),
                    serde_json::to_string(&r.payload)?,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Read every trace row for a program (dev tool; production uses the
    /// in-memory ring). Ordered by id ascending.
    ///
    /// # Errors
    /// Surfaces `SQLite` errors.
    pub fn read_traces(&self, program_id: ProgramId) -> StoreResult<Vec<TraceRecord>> {
        let mut stmt = self.conn().prepare(
            "SELECT tick, at, program_id, kind, payload_json
             FROM traces WHERE program_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([program_id.to_string()], |r| {
            let tick: i64 = r.get(0)?;
            let at: i64 = r.get(1)?;
            let pid_s: Option<String> = r.get(2)?;
            let kind_s: String = r.get(3)?;
            let payload_s: String = r.get(4)?;
            Ok(TraceRecord {
                tick: u64::try_from(tick).unwrap_or(0),
                at,
                program_id: pid_s.and_then(|s| s.parse().ok()),
                kind: match kind_s.as_str() {
                    "state_change" => TraceKind::StateChange,
                    "action_dispatched" => TraceKind::ActionDispatched,
                    "action_deferred" => TraceKind::ActionDeferred,
                    "watch_fired" => TraceKind::WatchFired,
                    "tick_skipped" => TraceKind::TickSkipped,
                    _ => TraceKind::PredicateEval,
                },
                payload: serde_json::from_str(&payload_s).unwrap_or(serde_json::Value::Null),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewProgram;
    use tempfile::tempdir;
    use vcli_core::state::ProgramState;

    #[test]
    fn empty_flush_is_noop() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        s.flush_traces(&[]).unwrap();
    }

    #[test]
    fn flush_and_read_roundtrip() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
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
        let rec = TraceRecord {
            tick: 7,
            at: 999,
            program_id: Some(id),
            kind: TraceKind::StateChange,
            payload: serde_json::json!({"from":"waiting","to":"running"}),
        };
        s.flush_traces(std::slice::from_ref(&rec)).unwrap();
        let back = s.read_traces(id).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].tick, 7);
        assert_eq!(back[0].kind, TraceKind::StateChange);
        assert_eq!(back[0].payload["to"], "running");
    }
}
