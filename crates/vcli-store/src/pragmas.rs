//! `SQLite` PRAGMAs applied to every `Store` connection per Decision 4.4:
//! WAL mode, NORMAL sync, 5s busy timeout, 32MB cache, FKs on, MEMORY temp store.

use rusqlite::Connection;

use crate::error::StoreResult;

/// Apply all PRAGMAs mandated by Decision 4.4. Order matters: `journal_mode`
/// before `synchronous` is the documented `SQLite` pattern.
///
/// # Errors
/// Surfaces `SQLite` errors.
pub fn apply_pragmas(conn: &Connection) -> StoreResult<()> {
    // journal_mode returns the new mode as a row; use query_row.
    let mode: String = conn.query_row("PRAGMA journal_mode=WAL;", [], |r| r.get(0))?;
    debug_assert_eq!(mode.to_lowercase(), "wal");
    conn.execute_batch(
        "PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA cache_size = -32000;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn one_line_pragma(conn: &Connection, pragma: &str) -> String {
        conn.query_row(&format!("PRAGMA {pragma};"), [], |r| {
            // Columns may be text or integer; stringify uniformly.
            let v: rusqlite::types::Value = r.get(0)?;
            Ok(match v {
                rusqlite::types::Value::Text(s) => s,
                rusqlite::types::Value::Integer(i) => i.to_string(),
                rusqlite::types::Value::Real(f) => f.to_string(),
                rusqlite::types::Value::Null => "null".to_string(),
                rusqlite::types::Value::Blob(_) => "<blob>".to_string(),
            })
        })
        .unwrap()
    }

    #[test]
    fn wal_mode_enabled_on_file_backed_db() {
        let d = tempdir().unwrap();
        let conn = Connection::open(d.path().join("t.db")).unwrap();
        apply_pragmas(&conn).unwrap();
        assert_eq!(one_line_pragma(&conn, "journal_mode").to_lowercase(), "wal");
    }

    #[test]
    fn foreign_keys_on() {
        let d = tempdir().unwrap();
        let conn = Connection::open(d.path().join("t.db")).unwrap();
        apply_pragmas(&conn).unwrap();
        assert_eq!(one_line_pragma(&conn, "foreign_keys"), "1");
    }

    #[test]
    fn busy_timeout_is_5000ms() {
        let d = tempdir().unwrap();
        let conn = Connection::open(d.path().join("t.db")).unwrap();
        apply_pragmas(&conn).unwrap();
        assert_eq!(one_line_pragma(&conn, "busy_timeout"), "5000");
    }
}
