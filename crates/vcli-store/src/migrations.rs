//! Hand-written linear migrations. Each `&str` in `MIGRATIONS` is applied
//! exactly once, in order, inside a single transaction. `schema_version`
//! holds the highest applied version.
//!
//! Authoritative schema source: spec §Persistence → `SQLite` schema (v0).
//! Spec notes the `programs` / `program_assets` / `events` / `schema_version`
//! tables. We extend v0 with `traces` (opt-in flush-on-shutdown ring) and
//! an `assets` index so `put_asset`/`get_asset` have a queryable hash table.

use rusqlite::{Connection, Transaction};

use crate::error::{StoreError, StoreResult};

/// The highest schema version this binary knows how to produce.
pub const LATEST_SCHEMA_VERSION: u32 = 1;

/// One SQL string per version. Index `i` migrates the db FROM version `i`
/// (empty or at `i`) TO version `i+1`. First element (index 0) is the bootstrap.
const MIGRATIONS: &[&str] = &[
    // ---- version 1: v0 bootstrap ------------------------------------------
    r"
    CREATE TABLE IF NOT EXISTS schema_version (
        version INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS programs (
        id              TEXT PRIMARY KEY,
        name            TEXT NOT NULL,
        source_json     TEXT NOT NULL,
        state           TEXT NOT NULL,
        submitted_at    INTEGER NOT NULL,
        started_at      INTEGER,
        finished_at     INTEGER,
        last_error_code TEXT,
        last_error_msg  TEXT,
        labels_json     TEXT NOT NULL DEFAULT '{}',
        body_cursor     INTEGER NOT NULL DEFAULT 0,
        body_entered_at INTEGER
    );
    CREATE INDEX IF NOT EXISTS programs_state_idx ON programs(state);

    CREATE TABLE IF NOT EXISTS program_assets (
        program_id  TEXT NOT NULL REFERENCES programs(id) ON DELETE CASCADE,
        asset_hash  TEXT NOT NULL,
        PRIMARY KEY (program_id, asset_hash)
    );
    CREATE INDEX IF NOT EXISTS program_assets_hash_idx ON program_assets(asset_hash);

    CREATE TABLE IF NOT EXISTS events (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        program_id  TEXT NOT NULL REFERENCES programs(id) ON DELETE CASCADE,
        type        TEXT NOT NULL,
        data_json   TEXT NOT NULL,
        at          INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS events_program_idx ON events(program_id);
    CREATE INDEX IF NOT EXISTS events_at_idx      ON events(at);

    CREATE TABLE IF NOT EXISTS assets (
        hash       TEXT PRIMARY KEY,
        byte_len   INTEGER NOT NULL,
        extension  TEXT,
        added_at   INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS traces (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        program_id  TEXT,
        tick        INTEGER NOT NULL,
        at          INTEGER NOT NULL,
        kind        TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        FOREIGN KEY (program_id) REFERENCES programs(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS traces_program_idx ON traces(program_id);
    CREATE INDEX IF NOT EXISTS traces_at_idx      ON traces(at);
    ",
];

/// Read current version from `schema_version`. Returns `0` if table is empty
/// or missing.
///
/// # Errors
/// Surfaces `SQLite` errors.
pub fn current_version(conn: &Connection) -> StoreResult<u32> {
    // Create table lazily so this works on a brand-new db without panic.
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")?;
    let v: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .ok();
    Ok(u32::try_from(v.unwrap_or(0)).unwrap_or(0))
}

/// Run every pending migration. Idempotent — re-running on an up-to-date db
/// is a no-op. Errors if the db is ahead of this binary.
///
/// # Errors
/// `StoreError::SchemaNewer` if the db version exceeds the latest supported version.
/// Surfaces `SQLite` errors on any other failure.
pub fn run_migrations(conn: &mut Connection) -> StoreResult<()> {
    let cur = current_version(conn)?;
    if u32::try_from(MIGRATIONS.len()).unwrap_or(0) < cur {
        return Err(StoreError::SchemaNewer {
            found: cur,
            supported: LATEST_SCHEMA_VERSION,
        });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let target = u32::try_from(i + 1).unwrap_or(u32::MAX);
        if target <= cur {
            continue;
        }
        let tx: Transaction<'_> = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute("INSERT INTO schema_version (version) VALUES (?1)", [target])?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn memory() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn fresh_db_starts_at_version_zero() {
        let conn = memory();
        assert_eq!(current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn run_migrations_goes_to_latest() {
        let mut conn = memory();
        run_migrations(&mut conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), LATEST_SCHEMA_VERSION);
    }

    #[test]
    fn run_migrations_is_idempotent() {
        let mut conn = memory();
        run_migrations(&mut conn).unwrap();
        run_migrations(&mut conn).unwrap();
        run_migrations(&mut conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), LATEST_SCHEMA_VERSION);
        // schema_version row count equals latest version (one per applied migration).
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(u32::try_from(n).unwrap(), LATEST_SCHEMA_VERSION);
    }

    #[test]
    fn all_v0_tables_exist_after_migration() {
        let mut conn = memory();
        run_migrations(&mut conn).unwrap();
        for table in [
            "programs",
            "program_assets",
            "events",
            "assets",
            "traces",
            "schema_version",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "table {table} missing");
        }
    }

    #[test]
    fn schema_newer_error_on_db_ahead_of_binary() {
        let mut conn = memory();
        run_migrations(&mut conn).unwrap();
        // Pretend a future version wrote a row.
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [LATEST_SCHEMA_VERSION + 5],
        )
        .unwrap();
        let err = run_migrations(&mut conn).unwrap_err();
        assert!(matches!(err, StoreError::SchemaNewer { .. }));
    }
}
