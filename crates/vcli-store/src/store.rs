//! `Store` — the top-level handle owning the `SQLite` connection.
//!
//! Opening performs: mkdir `data_root` + `assets_root`, connect + apply PRAGMAs,
//! run migrations, and perform restart recovery (any row in state `running`
//! transitions to `failed(daemon_restart)`, preserving `body_cursor`).
//! See spec §Restart semantics, step 4.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use vcli_core::ids::ProgramId;
use vcli_core::state::ProgramState;

use crate::error::{StoreError, StoreResult};
use crate::migrations::run_migrations;
use crate::paths::{assets_root, db_path};
use crate::pragmas::apply_pragmas;

/// A program that was found in `running` at startup and rewritten to `failed`.
/// Returned from `Store::open` so the runtime can emit the corresponding
/// `program.state_changed` / `program.failed` events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredProgram {
    /// The program id.
    pub id: ProgramId,
    /// Cursor preserved for opt-in `vcli resume`.
    pub body_cursor: u32,
}

/// Handle to the on-disk store. Single-writer at a time; `SQLite` WAL mode
/// supports concurrent readers (see `wal_concurrent.rs` integration test).
pub struct Store {
    data_root: PathBuf,
    conn: Connection,
}

impl Store {
    /// Open (or create) the store rooted at `data_root`.
    ///
    /// # Errors
    /// Surfaces IO, `SQLite`, and migration failures.
    pub fn open(data_root: impl AsRef<Path>) -> StoreResult<(Self, Vec<RecoveredProgram>)> {
        let data_root = data_root.as_ref().to_path_buf();

        // 1. Ensure directory exists.
        fs::create_dir_all(&data_root).map_err(|e| StoreError::Io {
            path: data_root.clone(),
            source: e,
        })?;
        let assets = assets_root(&data_root);
        fs::create_dir_all(&assets).map_err(|e| StoreError::Io {
            path: assets.clone(),
            source: e,
        })?;

        // 2. Connect + PRAGMAs + migrations.
        let mut conn = Connection::open(db_path(&data_root))?;
        apply_pragmas(&conn)?;
        run_migrations(&mut conn)?;

        // 3. Restart recovery.
        let recovered = recover_running_programs(&mut conn)?;

        Ok((Self { data_root, conn }, recovered))
    }

    /// Data root this store was opened against.
    #[must_use]
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// Read-only access to the underlying connection (integration tests + gc).
    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Insert a new program row. `source_json` MUST already be canonicalized
    /// (the caller uses `vcli_core::canonicalize` beforehand — see Decision 1.1).
    ///
    /// # Errors
    /// Fails if a row with the same id already exists.
    pub fn insert_program(&mut self, row: &NewProgram<'_>) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO programs
                (id, name, source_json, state, submitted_at, labels_json, body_cursor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            rusqlite::params![
                row.id.to_string(),
                row.name,
                row.source_json,
                row.state.as_str(),
                row.submitted_at,
                row.labels_json,
            ],
        )?;
        Ok(())
    }

    /// Fetch a program row by id.
    ///
    /// # Errors
    /// `StoreError::UnknownProgram` if the id is not in the table.
    ///
    /// # Panics
    /// Panics if a stored id or state is not a valid value (should never happen
    /// for data written by this crate).
    pub fn get_program(&self, id: ProgramId) -> StoreResult<ProgramRow> {
        self.conn
            .query_row(
                "SELECT id, name, source_json, state, submitted_at, started_at,
                        finished_at, last_error_code, last_error_msg, labels_json,
                        body_cursor, body_entered_at
                 FROM programs WHERE id = ?1",
                [id.to_string()],
                |r| {
                    Ok(ProgramRow {
                        id: r.get::<_, String>(0)?.parse().unwrap(),
                        name: r.get(1)?,
                        source_json: r.get(2)?,
                        state: r
                            .get::<_, String>(3)?
                            .parse()
                            .unwrap_or(ProgramState::Pending),
                        submitted_at: r.get(4)?,
                        started_at: r.get(5)?,
                        finished_at: r.get(6)?,
                        last_error_code: r.get(7)?,
                        last_error_msg: r.get(8)?,
                        labels_json: r.get(9)?,
                        body_cursor: u32::try_from(r.get::<_, i64>(10)?).unwrap_or(0),
                        body_entered_at: r.get(11)?,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StoreError::UnknownProgram(id.to_string()),
                other => StoreError::Sqlite(other),
            })
    }

    /// Update state of a program. If transitioning to a terminal state, set
    /// `finished_at`. If transitioning from `waiting` to `running`, set
    /// `started_at`. Caller provides `now_ms` to avoid reading a clock here.
    ///
    /// # Errors
    /// `StoreError::UnknownProgram` if the id is not present.
    pub fn update_state(
        &mut self,
        id: ProgramId,
        new_state: ProgramState,
        now_ms: i64,
    ) -> StoreResult<()> {
        let n = self.conn.execute(
            "UPDATE programs
                SET state = ?1,
                    started_at = COALESCE(started_at, CASE WHEN ?1 = 'running' THEN ?2 ELSE NULL END),
                    finished_at = CASE WHEN ?1 IN ('completed','failed','cancelled') THEN ?2 ELSE finished_at END
              WHERE id = ?3",
            rusqlite::params![new_state.as_str(), now_ms, id.to_string()],
        )?;
        if n == 0 {
            return Err(StoreError::UnknownProgram(id.to_string()));
        }
        Ok(())
    }

    /// Advance `body_cursor` to `cursor`. Called after each body step resolves.
    ///
    /// # Errors
    /// `StoreError::UnknownProgram` if the id is not present.
    pub fn set_body_cursor(&mut self, id: ProgramId, cursor: u32) -> StoreResult<()> {
        let n = self.conn.execute(
            "UPDATE programs SET body_cursor = ?1 WHERE id = ?2",
            rusqlite::params![i64::from(cursor), id.to_string()],
        )?;
        if n == 0 {
            return Err(StoreError::UnknownProgram(id.to_string()));
        }
        Ok(())
    }

    /// Record the last error (code + message) for a program, without changing state.
    ///
    /// # Errors
    /// Surfaces `SQLite` errors.
    pub fn set_last_error(&mut self, id: ProgramId, code: &str, msg: &str) -> StoreResult<()> {
        self.conn.execute(
            "UPDATE programs SET last_error_code = ?1, last_error_msg = ?2 WHERE id = ?3",
            rusqlite::params![code, msg, id.to_string()],
        )?;
        Ok(())
    }

    /// List programs, optionally filtered by a single [`ProgramState`].
    /// Ordered by `submitted_at ASC` then by rowid for stability.
    ///
    /// # Errors
    /// Surfaces `SQLite` errors.
    ///
    /// # Panics
    /// Panics if a stored id or state value cannot be parsed (should never
    /// happen for rows written by this crate).
    pub fn list_programs(
        &self,
        state_filter: Option<ProgramState>,
    ) -> StoreResult<Vec<ProgramRow>> {
        let state_str = state_filter.map(|s| s.as_str().to_string());
        let mut stmt = self.conn.prepare(
            "SELECT id, name, source_json, state, submitted_at, started_at,
                    finished_at, last_error_code, last_error_msg, labels_json,
                    body_cursor, body_entered_at
             FROM programs
             WHERE (?1 IS NULL) OR (state = ?1)
             ORDER BY submitted_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![state_str], |r| {
            let id: String = r.get(0)?;
            let state: String = r.get(3)?;
            let body_cursor: i64 = r.get(10)?;
            Ok(ProgramRow {
                id: id.parse().unwrap(),
                name: r.get(1)?,
                source_json: r.get(2)?,
                state: state.parse().unwrap(),
                submitted_at: r.get(4)?,
                started_at: r.get(5)?,
                finished_at: r.get(6)?,
                last_error_code: r.get(7)?,
                last_error_msg: r.get(8)?,
                labels_json: r.get(9)?,
                body_cursor: u32::try_from(body_cursor).unwrap_or(0),
                body_entered_at: r.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

/// Values a caller supplies when inserting a new program row.
pub struct NewProgram<'a> {
    /// Program id.
    pub id: ProgramId,
    /// Program name.
    pub name: &'a str,
    /// Canonical-form source JSON (see `vcli_core::canonicalize`).
    pub source_json: &'a str,
    /// Initial state (normally `ProgramState::Pending`).
    pub state: ProgramState,
    /// Unix ms of submission.
    pub submitted_at: i64,
    /// JSON-encoded labels (pass `"{}"` when none).
    pub labels_json: &'a str,
}

/// Snapshot of a row in `programs` returned by `get_program`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramRow {
    /// Program id.
    pub id: ProgramId,
    /// Human name.
    pub name: String,
    /// Canonical JSON source.
    pub source_json: String,
    /// Current state.
    pub state: ProgramState,
    /// Submission time (unix ms).
    pub submitted_at: i64,
    /// When body execution started (unix ms), if at all.
    pub started_at: Option<i64>,
    /// When the program reached a terminal state, if at all.
    pub finished_at: Option<i64>,
    /// Last error code, if any.
    pub last_error_code: Option<String>,
    /// Last error message, if any.
    pub last_error_msg: Option<String>,
    /// JSON-encoded labels.
    pub labels_json: String,
    /// Next body step index to execute.
    pub body_cursor: u32,
    /// When body started (unix ms).
    pub body_entered_at: Option<i64>,
}

fn recover_running_programs(conn: &mut Connection) -> StoreResult<Vec<RecoveredProgram>> {
    let tx = conn.transaction()?;
    let mut recovered = Vec::new();
    {
        let mut stmt =
            tx.prepare("SELECT id, body_cursor FROM programs WHERE state = 'running'")?;
        let rows = stmt.query_map([], |r| {
            let id_str: String = r.get(0)?;
            let cursor: i64 = r.get(1)?;
            Ok((id_str, cursor))
        })?;
        for row in rows {
            let (id_str, cursor) = row?;
            let id: ProgramId = id_str
                .parse()
                .map_err(|_| StoreError::UnknownProgram(id_str.clone()))?;
            recovered.push(RecoveredProgram {
                id,
                body_cursor: u32::try_from(cursor).unwrap_or(0),
            });
        }
    }
    if !recovered.is_empty() {
        tx.execute(
            "UPDATE programs
               SET state = ?1,
                   last_error_code = 'daemon_restart',
                   last_error_msg  = 'daemon restarted during execution'
             WHERE state = 'running'",
            [ProgramState::Failed.as_str()],
        )?;
    }
    tx.commit()?;
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_data_root_and_assets_dir() {
        let d = tempdir().unwrap();
        let root = d.path().join("nested").join("vcli");
        let (store, recovered) = Store::open(&root).unwrap();
        assert!(root.exists());
        assert!(root.join("assets").join("sha256").exists());
        assert_eq!(recovered, vec![]);
        assert_eq!(store.data_root(), root);
    }

    #[test]
    fn open_is_idempotent() {
        let d = tempdir().unwrap();
        let _ = Store::open(d.path()).unwrap();
        let _ = Store::open(d.path()).unwrap();
        // Ensure reopen didn't rerun migrations beyond latest.
    }

    fn new_program_row(id: ProgramId, name: &str) -> NewProgram<'_> {
        NewProgram {
            id,
            name,
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 1_000,
            labels_json: "{}",
        }
    }

    #[test]
    fn insert_and_get_program_roundtrip() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = ProgramId::new();
        s.insert_program(&new_program_row(id, "yt")).unwrap();
        let row = s.get_program(id).unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.name, "yt");
        assert_eq!(row.state, ProgramState::Pending);
        assert_eq!(row.body_cursor, 0);
    }

    #[test]
    fn get_program_unknown_id_errors() {
        let d = tempdir().unwrap();
        let (s, _) = Store::open(d.path()).unwrap();
        let id = ProgramId::new();
        let err = s.get_program(id).unwrap_err();
        assert!(matches!(err, StoreError::UnknownProgram(_)));
    }

    #[test]
    fn update_state_writes_started_and_finished_at() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = ProgramId::new();
        s.insert_program(&new_program_row(id, "x")).unwrap();
        s.update_state(id, ProgramState::Running, 5_000).unwrap();
        let r = s.get_program(id).unwrap();
        assert_eq!(r.state, ProgramState::Running);
        assert_eq!(r.started_at, Some(5_000));
        assert_eq!(r.finished_at, None);

        s.update_state(id, ProgramState::Completed, 9_000).unwrap();
        let r = s.get_program(id).unwrap();
        assert_eq!(r.state, ProgramState::Completed);
        assert_eq!(r.started_at, Some(5_000));
        assert_eq!(r.finished_at, Some(9_000));
    }

    #[test]
    fn set_body_cursor_persists() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = ProgramId::new();
        s.insert_program(&new_program_row(id, "x")).unwrap();
        s.set_body_cursor(id, 7).unwrap();
        assert_eq!(s.get_program(id).unwrap().body_cursor, 7);
    }

    #[test]
    fn list_programs_returns_all_when_no_filter() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
            s.insert_program(&NewProgram {
                id: ProgramId::new(),
                name,
                source_json: "{}",
                state: ProgramState::Pending,
                submitted_at: i as i64,
                labels_json: "{}",
            })
            .unwrap();
        }
        let all = s.list_programs(None).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].name, "a");
    }

    #[test]
    fn list_programs_filters_by_state() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let running = ProgramId::new();
        let pending = ProgramId::new();
        for (id, name) in [(running, "r"), (pending, "p")] {
            s.insert_program(&NewProgram {
                id,
                name,
                source_json: "{}",
                state: ProgramState::Pending,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
        }
        s.update_state(running, ProgramState::Running, 100).unwrap();
        let runs = s.list_programs(Some(ProgramState::Running)).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].name, "r");
    }

    #[test]
    fn reopen_preserves_rows() {
        let d = tempdir().unwrap();
        let id = ProgramId::new();
        {
            let (mut s, _) = Store::open(d.path()).unwrap();
            s.insert_program(&new_program_row(id, "x")).unwrap();
        }
        let (s, _) = Store::open(d.path()).unwrap();
        assert_eq!(s.get_program(id).unwrap().name, "x");
    }
}
