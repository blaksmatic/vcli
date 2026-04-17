//! Transition a program from `failed(daemon_restart)` back into `running`.
//! Eligibility (no watches, no `elapsed_ms_since_true`, no `sleep_ms` step,
//! no throttled-and-fired watch, no unresolved postcondition — spec §Resume
//! and Decision C) is decided by the caller; this module is purely the DB
//! transition.

use vcli_core::ids::ProgramId;
use vcli_core::state::ProgramState;

use crate::error::{StoreError, StoreResult};
use crate::store::Store;

/// The cursor at which body should resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeOutcome {
    /// Body step index to resume at.
    pub from_step: u32,
}

impl Store {
    /// Transition a program from `failed(daemon_restart)` back to `running`.
    /// Returns the `body_cursor` (zero if `from_start` is true).
    ///
    /// # Errors
    /// - `StoreError::UnknownProgram` if the id is not present.
    /// - `StoreError::NotResumable` if the row is not in `failed(daemon_restart)`.
    pub fn resume_program(
        &mut self,
        id: ProgramId,
        from_start: bool,
        now_ms: i64,
    ) -> StoreResult<ResumeOutcome> {
        let tx = self.conn_mut().transaction()?;
        // 1. Read current row.
        let (state, code, cursor): (String, Option<String>, i64) = tx
            .query_row(
                "SELECT state, last_error_code, body_cursor FROM programs WHERE id = ?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StoreError::UnknownProgram(id.to_string()),
                other => StoreError::Sqlite(other),
            })?;

        if state != ProgramState::Failed.as_str() || code.as_deref() != Some("daemon_restart") {
            return Err(StoreError::NotResumable(format!(
                "program {id} state={state} code={code:?}"
            )));
        }

        let new_cursor = if from_start {
            0
        } else {
            u32::try_from(cursor).unwrap_or(0)
        };

        tx.execute(
            "UPDATE programs
                SET state = ?1,
                    last_error_code = NULL,
                    last_error_msg  = NULL,
                    body_cursor = ?2,
                    finished_at = NULL
              WHERE id = ?3",
            rusqlite::params![
                ProgramState::Running.as_str(),
                i64::from(new_cursor),
                id.to_string()
            ],
        )?;
        // Started_at: only set if null.
        tx.execute(
            "UPDATE programs SET started_at = COALESCE(started_at, ?1) WHERE id = ?2",
            rusqlite::params![now_ms, id.to_string()],
        )?;
        tx.commit()?;
        Ok(ResumeOutcome {
            from_step: new_cursor,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewProgram;
    use tempfile::tempdir;
    use vcli_core::state::ProgramState;

    fn seed_failed_daemon_restart(s: &mut Store, id: ProgramId, cursor: u32) {
        s.insert_program(&NewProgram {
            id,
            name: "x",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
        s.update_state(id, ProgramState::Running, 100).unwrap();
        s.set_body_cursor(id, cursor).unwrap();
        // Simulate the daemon-restart recovery path.
        let _ = Store::open(s.data_root()).unwrap();
    }

    #[test]
    fn resume_preserves_cursor_by_default() {
        let d = tempdir().unwrap();
        let id = ProgramId::new();
        let (mut s, _) = Store::open(d.path()).unwrap();
        seed_failed_daemon_restart(&mut s, id, 4);
        let (mut s, _) = Store::open(d.path()).unwrap(); // reopen post-recovery
        let out = s.resume_program(id, false, 200).unwrap();
        assert_eq!(out.from_step, 4);
        let r = s.get_program(id).unwrap();
        assert_eq!(r.state, ProgramState::Running);
        assert_eq!(r.body_cursor, 4);
        assert!(r.last_error_code.is_none());
    }

    #[test]
    fn resume_from_start_resets_cursor() {
        let d = tempdir().unwrap();
        let id = ProgramId::new();
        let (mut s, _) = Store::open(d.path()).unwrap();
        seed_failed_daemon_restart(&mut s, id, 9);
        let (mut s, _) = Store::open(d.path()).unwrap();
        let out = s.resume_program(id, true, 200).unwrap();
        assert_eq!(out.from_step, 0);
        assert_eq!(s.get_program(id).unwrap().body_cursor, 0);
    }

    #[test]
    fn resume_rejects_non_daemon_restart_failure() {
        let d = tempdir().unwrap();
        let id = ProgramId::new();
        let (mut s, _) = Store::open(d.path()).unwrap();
        s.insert_program(&NewProgram {
            id,
            name: "x",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
        s.update_state(id, ProgramState::Failed, 10).unwrap();
        s.set_last_error(id, "wait_for_timeout", "timed out")
            .unwrap();
        let err = s.resume_program(id, false, 11).unwrap_err();
        assert!(matches!(err, StoreError::NotResumable(_)));
    }

    #[test]
    fn resume_unknown_program_errors() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let err = s.resume_program(ProgramId::new(), false, 0).unwrap_err();
        assert!(matches!(err, StoreError::UnknownProgram(_)));
    }
}
