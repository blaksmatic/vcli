//! `Store` — the top-level handle owning the SQLite connection.
//!
//! Opening performs: mkdir data_root + assets_root, connect + apply PRAGMAs,
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

/// Handle to the on-disk store. Single-writer at a time; SQLite WAL mode
/// supports concurrent readers (see `wal_concurrent.rs` integration test).
pub struct Store {
    data_root: PathBuf,
    conn: Connection,
}

impl Store {
    /// Open (or create) the store rooted at `data_root`.
    ///
    /// # Errors
    /// Surfaces IO, SQLite, and migration failures.
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
}

fn recover_running_programs(conn: &mut Connection) -> StoreResult<Vec<RecoveredProgram>> {
    let tx = conn.transaction()?;
    let mut recovered = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id, body_cursor FROM programs WHERE state = 'running'",
        )?;
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
}
