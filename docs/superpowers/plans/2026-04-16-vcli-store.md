# vcli-store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SQLite-backed persistence for programs/events/traces plus a content-addressed asset store, with restart recovery and 7-day GC.

**Architecture:** `vcli-store` is a synchronous library crate backed by `rusqlite` (bundled SQLite). Chosen over `sqlx` because the daemon runtime is sync on the tick thread (tokio is IPC-only), `rusqlite` keeps compile time + binary size small, and a hand-written linear `schema_version` migration table stays in lockstep with the design spec §Persistence. The `AssetStore` writes content-addressed blobs to `<root>/assets/sha256/<aa>/<bb>/<hex>.<ext>` (spec §Asset store) — writes are atomic (temp file + rename), reads are by hash, and the `program_assets` table joins program rows to blob hashes for reference-counted GC. Restart recovery runs during `Store::open`: any row left in `running` is transitioned to `failed(daemon_restart)` in a single transaction, preserving `body_cursor` for opt-in `vcli resume`. PRAGMAs (WAL + NORMAL + 5s busy timeout + FKs + 32MB cache + memory temp store) are applied on every connection open per Decision 4.4.

**Tech Stack:** Rust 2021, `rusqlite` 0.31 with `bundled` feature, `sha2` 0.10 (for asset hashing — distinct from `vcli-core::canonical`'s minimal in-crate sha256 used for `PredicateHash`), `thiserror`, `serde`, `serde_json`, `tempfile` (dev), `proptest` (dev). Depends on `vcli-core` (`Program`, `ProgramId`, `ProgramState`, `Event`, `EventData`, `UnixMs`, `canonicalize`).

---

## File structure produced by this plan

```
vcli/
├── Cargo.toml                        # modify: add vcli-store to workspace members + deps
└── crates/
    └── vcli-store/
        ├── Cargo.toml                # crate manifest
        ├── src/
        │   ├── lib.rs                # module tree + re-exports
        │   ├── error.rs              # StoreError + StoreResult
        │   ├── paths.rs              # layout helpers (data dir, assets dir, sharded blob path)
        │   ├── migrations.rs         # linear migrations + `run_migrations(conn)`
        │   ├── pragmas.rs            # apply_pragmas(conn) — WAL + NORMAL + FKs + cache
        │   ├── store.rs              # Store::open, programs CRUD, state transitions, restart recovery
        │   ├── events.rs             # append_event + stream_events(since)
        │   ├── traces.rs             # TraceRecord persistence (flush-on-demand ring)
        │   ├── assets.rs             # AssetHash newtype + put_asset + get_asset + list_referenced
        │   ├── gc.rs                 # gc_programs(older_than) + gc_assets() + daemon startup GC trigger
        │   └── resume.rs             # resume_program (failed(daemon_restart) → running)
        └── tests/
            ├── restart_recovery.rs   # integration: running → failed(daemon_restart) on open
            ├── assets_dedup.rs       # integration: same bytes → same hash + path
            ├── wal_concurrent.rs     # integration: two readers against one writer
            └── gc.rs                 # integration: 7-day prune keeps recent, removes stale
```

**Responsibility split rationale:** `store.rs` owns the `Store` handle and programs table; `events.rs`, `traces.rs`, `assets.rs`, `gc.rs`, `resume.rs` each own one concept. `migrations.rs` and `pragmas.rs` are boring infrastructure isolated for unit testability. No file exceeds ~350 lines.

---

## Task 1: Workspace wiring + empty `vcli-store` crate

**Files:**
- Modify: `/Users/admin/Workspace/vcli/Cargo.toml`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/Cargo.toml`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Add workspace dependencies to the root `Cargo.toml`**

Under `[workspace.dependencies]` append:

```toml
rusqlite = { version = "0.31", features = ["bundled"] }
sha2 = "0.10"
tempfile = "3"
hex = "0.4"
```

Under `[workspace]` → `members` append `"crates/vcli-store"`:

```toml
members = [
    "crates/vcli-core",
    "crates/vcli-store",
]
```

- [ ] **Step 2: Create `crates/vcli-store/Cargo.toml`**

```toml
[package]
name = "vcli-store"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "SQLite persistence + content-addressed asset store for vcli."

[dependencies]
vcli-core = { path = "../vcli-core" }
rusqlite = { workspace = true }
sha2 = { workspace = true }
hex = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
proptest = { workspace = true }
```

- [ ] **Step 3: Create `crates/vcli-store/src/lib.rs`**

```rust
//! vcli-store — SQLite persistence + content-addressed asset store.
//!
//! See the v0 design spec at `docs/superpowers/specs/2026-04-16-vcli-design.md`
//! §Persistence for the authoritative schema and semantics this crate implements.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
```

- [ ] **Step 4: Verify build**

Run: `cargo check -p vcli-store`
Expected: OK.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/vcli-store/Cargo.toml crates/vcli-store/src/lib.rs
git commit -m "vcli-store: empty crate shell + workspace wiring"
```

---

## Task 2: `StoreError` + `StoreResult`

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/error.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module + re-export to `lib.rs`**

```rust
pub mod error;

pub use error::{StoreError, StoreResult};
```

- [ ] **Step 2: Write `error.rs` (tests first)**

```rust
//! Error types for the store.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Error surface for every fallible `Store` operation.
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite returned an error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Filesystem IO error (asset store, db path creation).
    #[error("io at {path:?}: {source}")]
    Io {
        /// Path that triggered the IO error, if known.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: io::Error,
    },

    /// A program row was not found.
    #[error("unknown program: {0}")]
    UnknownProgram(String),

    /// Requested state transition is not allowed from the current state.
    #[error("bad state transition: {from} -> {to} ({reason})")]
    BadStateTransition {
        /// Starting state.
        from: String,
        /// Requested ending state.
        to: String,
        /// Human-readable reason.
        reason: String,
    },

    /// Program was not failed(daemon_restart) or is otherwise not resumable.
    #[error("not resumable: {0}")]
    NotResumable(String),

    /// Asset bytes not found for the given hash.
    #[error("unknown asset: {0}")]
    UnknownAsset(String),

    /// Schema version in the db is newer than this binary understands.
    #[error("schema version {found} is newer than supported version {supported}")]
    SchemaNewer {
        /// Version read from the db.
        found: u32,
        /// Highest version baked into this binary.
        supported: u32,
    },

    /// A stored JSON blob failed to deserialize.
    #[error("json deserialize: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience alias.
pub type StoreResult<T> = Result<T, StoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_program_display() {
        let e = StoreError::UnknownProgram("abc".into());
        assert_eq!(e.to_string(), "unknown program: abc");
    }

    #[test]
    fn bad_state_transition_carries_fields() {
        let e = StoreError::BadStateTransition {
            from: "completed".into(),
            to: "running".into(),
            reason: "terminal".into(),
        };
        assert!(e.to_string().contains("completed -> running"));
    }

    #[test]
    fn schema_newer_display() {
        let e = StoreError::SchemaNewer { found: 5, supported: 3 };
        assert!(e.to_string().contains("5"));
        assert!(e.to_string().contains("3"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib error`
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/error.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: StoreError + StoreResult"
```

---

## Task 3: Path helpers (data dir, asset sharding)

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/paths.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append `pub mod paths;` + re-export to `lib.rs`**

```rust
pub mod paths;

pub use paths::{asset_blob_path, assets_root, db_path};
```

- [ ] **Step 2: Write `paths.rs`**

```rust
//! Filesystem layout helpers.
//!
//! Per spec §Persistence → Data layout (macOS v0):
//!   ~/Library/Application Support/vcli/
//!   ├── vcli.db
//!   └── assets/sha256/ab/cd/abcd…ef.png
//!
//! Callers pass in a `data_root`; helpers never touch `$HOME` directly so
//! every test drives the layout from a `tempdir()`.

use std::path::{Path, PathBuf};

/// Absolute path to the SQLite database under `data_root`.
#[must_use]
pub fn db_path(data_root: &Path) -> PathBuf {
    data_root.join("vcli.db")
}

/// Absolute path to the assets root under `data_root`: `<data_root>/assets/sha256/`.
#[must_use]
pub fn assets_root(data_root: &Path) -> PathBuf {
    data_root.join("assets").join("sha256")
}

/// Path to the blob for `hex_hash`, inside `assets_root(data_root)`, with optional
/// extension. Layout: `assets/sha256/<xx>/<yy>/<full_hex><.ext>` where `xx` are the
/// first two hex chars and `yy` the next two.
///
/// `extension` must NOT include the leading dot.
#[must_use]
pub fn asset_blob_path(data_root: &Path, hex_hash: &str, extension: Option<&str>) -> PathBuf {
    assert!(
        hex_hash.len() >= 4,
        "hex hash too short ({} chars), must be ≥ 4 for sharding",
        hex_hash.len()
    );
    let root = assets_root(data_root);
    let xx = &hex_hash[..2];
    let yy = &hex_hash[2..4];
    let file_name = match extension {
        Some(ext) if !ext.is_empty() => format!("{hex_hash}.{ext}"),
        _ => hex_hash.to_string(),
    };
    root.join(xx).join(yy).join(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn db_path_is_data_root_joined_vcli_db() {
        let p = db_path(Path::new("/root"));
        assert_eq!(p, PathBuf::from("/root/vcli.db"));
    }

    #[test]
    fn assets_root_is_sharded_by_sha256() {
        let p = assets_root(Path::new("/root"));
        assert_eq!(p, PathBuf::from("/root/assets/sha256"));
    }

    #[test]
    fn blob_path_uses_first_four_chars_as_two_shards() {
        let p = asset_blob_path(
            Path::new("/root"),
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            Some("png"),
        );
        assert_eq!(
            p,
            PathBuf::from(
                "/root/assets/sha256/ab/cd/abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789.png"
            )
        );
    }

    #[test]
    fn blob_path_without_extension() {
        let p = asset_blob_path(Path::new("/r"), "deadbeef" ,None);
        assert_eq!(p, PathBuf::from("/r/assets/sha256/de/ad/deadbeef"));
    }

    #[test]
    #[should_panic(expected = "hex hash too short")]
    fn blob_path_panics_on_short_hash() {
        let _ = asset_blob_path(Path::new("/r"), "ab", None);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib paths`
Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/paths.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: path helpers (db + sharded asset blob layout)"
```

---

## Task 4: PRAGMA application on connection open

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/pragmas.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module to `lib.rs`**

```rust
pub mod pragmas;
```

- [ ] **Step 2: Write `pragmas.rs`**

```rust
//! SQLite PRAGMAs applied to every `Store` connection per Decision 4.4:
//! WAL mode, NORMAL sync, 5s busy timeout, 32MB cache, FKs on, MEMORY temp store.

use rusqlite::Connection;

use crate::error::StoreResult;

/// Apply all PRAGMAs mandated by Decision 4.4. Order matters: `journal_mode`
/// before `synchronous` is the documented SQLite pattern.
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib pragmas`
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/pragmas.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: apply WAL + PRAGMAs on every connection (Decision 4.4)"
```

---

## Task 5: Linear migrations + `schema_version` table

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/migrations.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module to `lib.rs`**

```rust
pub mod migrations;

pub use migrations::LATEST_SCHEMA_VERSION;
```

- [ ] **Step 2: Write `migrations.rs` with the full v0 schema inline**

Schema implements spec §SQLite schema (v0) + the prompt's trace and asset-blob requirements. Because the spec says "INSERT INTO schema_version VALUES (1)", we seed `1` and check it.

```rust
//! Hand-written linear migrations. Each `&str` in `MIGRATIONS` is applied
//! exactly once, in order, inside a single transaction. `schema_version`
//! holds the highest applied version.
//!
//! Authoritative schema source: spec §Persistence → SQLite schema (v0).
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
    r#"
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
    "#,
];

/// Read current version from `schema_version`. Returns `0` if table is empty
/// or missing.
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
        for table in ["programs", "program_assets", "events", "assets", "traces", "schema_version"] {
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib migrations`
Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/migrations.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: linear migrations + v0 schema (programs/events/assets/traces)"
```

---

## Task 6: `Store::open` with restart recovery

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/store.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module + re-export to `lib.rs`**

```rust
pub mod store;

pub use store::{RecoveredProgram, Store};
```

- [ ] **Step 2: Write the skeleton of `store.rs`**

```rust
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

    pub(crate) fn conn(&self) -> &Connection {
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib store`
Expected: 2 tests pass (more coverage arrives with `insert_program`).

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/store.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: Store::open + restart recovery scaffolding"
```

---

## Task 7: Programs CRUD (`insert_program`, `get_program`, `update_state`)

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/store.rs`

- [ ] **Step 1: Extend `store.rs` with CRUD methods**

Add to `impl Store`:

```rust
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
                        state: r.get::<_, String>(3)?.parse().unwrap_or(ProgramState::Pending),
                        submitted_at: r.get(4)?,
                        started_at: r.get(5)?,
                        finished_at: r.get(6)?,
                        last_error_code: r.get(7)?,
                        last_error_msg: r.get(8)?,
                        labels_json: r.get(9)?,
                        body_cursor: r.get::<_, i64>(10)? as u32,
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
    pub fn set_body_cursor(&mut self, id: ProgramId, cursor: u32) -> StoreResult<()> {
        let n = self.conn.execute(
            "UPDATE programs SET body_cursor = ?1 WHERE id = ?2",
            rusqlite::params![cursor as i64, id.to_string()],
        )?;
        if n == 0 {
            return Err(StoreError::UnknownProgram(id.to_string()));
        }
        Ok(())
    }

    /// Record the last error (code + message) for a program, without changing state.
    pub fn set_last_error(&mut self, id: ProgramId, code: &str, msg: &str) -> StoreResult<()> {
        self.conn.execute(
            "UPDATE programs SET last_error_code = ?1, last_error_msg = ?2 WHERE id = ?3",
            rusqlite::params![code, msg, id.to_string()],
        )?;
        Ok(())
    }
```

Add these types (below `Store`):

```rust
/// Values a caller supplies when inserting a new program row.
pub struct NewProgram<'a> {
    /// Program id.
    pub id: ProgramId,
    /// Program name.
    pub name: &'a str,
    /// Canonical-form source JSON (see vcli_core::canonicalize).
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
```

Re-export in `lib.rs`:

```rust
pub use store::{NewProgram, ProgramRow, RecoveredProgram, Store};
```

- [ ] **Step 2: Add CRUD + recovery tests to `store.rs` tests module**

```rust
    fn new_program_row<'a>(id: ProgramId, name: &'a str) -> NewProgram<'a> {
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib store`
Expected: 7 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/store.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: programs CRUD (insert/get/update_state/set_body_cursor)"
```

---

## Task 8: Restart-recovery integration test

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/tests/restart_recovery.rs`

- [ ] **Step 1: Write integration test covering the open-time recovery path**

```rust
//! Integration: `Store::open` transitions any row left in `running` to
//! `failed(daemon_restart)` and returns the recovered programs with their
//! `body_cursor` preserved. Spec §Restart semantics, step 4.

use tempfile::tempdir;

use vcli_core::ids::ProgramId;
use vcli_core::state::ProgramState;
use vcli_store::{NewProgram, Store};

fn insert_running(store: &mut Store, id: ProgramId, cursor: u32) {
    store
        .insert_program(&NewProgram {
            id,
            name: "x",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
    store.update_state(id, ProgramState::Running, 100).unwrap();
    store.set_body_cursor(id, cursor).unwrap();
}

#[test]
fn running_programs_transition_to_failed_on_reopen() {
    let d = tempdir().unwrap();
    let id1 = ProgramId::new();
    let id2 = ProgramId::new();

    {
        let (mut s, _) = Store::open(d.path()).unwrap();
        insert_running(&mut s, id1, 3);
        insert_running(&mut s, id2, 0);
    }

    // Simulate a daemon restart: reopen the store.
    let (s, recovered) = Store::open(d.path()).unwrap();
    assert_eq!(recovered.len(), 2);

    let r1 = s.get_program(id1).unwrap();
    assert_eq!(r1.state, ProgramState::Failed);
    assert_eq!(r1.last_error_code.as_deref(), Some("daemon_restart"));
    assert_eq!(r1.body_cursor, 3); // cursor preserved

    let r2 = s.get_program(id2).unwrap();
    assert_eq!(r2.state, ProgramState::Failed);
    assert_eq!(r2.body_cursor, 0);
}

#[test]
fn second_reopen_after_recovery_is_a_no_op() {
    let d = tempdir().unwrap();
    let id = ProgramId::new();
    {
        let (mut s, _) = Store::open(d.path()).unwrap();
        insert_running(&mut s, id, 5);
    }
    let (_, recovered_first) = Store::open(d.path()).unwrap();
    assert_eq!(recovered_first.len(), 1);

    let (_, recovered_second) = Store::open(d.path()).unwrap();
    assert!(recovered_second.is_empty(), "no rows still in running");
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p vcli-store --test restart_recovery`
Expected: 2 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-store/tests/restart_recovery.rs
git commit -m "vcli-store: integration test for running→failed(daemon_restart) on open"
```

---

## Task 9: `resume_program` (failed(daemon_restart) → running)

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/resume.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module + re-export**

In `lib.rs`:

```rust
pub mod resume;

pub use resume::ResumeOutcome;
```

- [ ] **Step 2: Write `resume.rs`**

Per spec §Resume, resume is "opt-in" and the caller (runtime) decides eligibility
based on program shape — that check does not live here. This module only provides
the state-transition helper: `failed(daemon_restart)` → `running`, returning the
preserved `body_cursor`. `--from-start` resets the cursor to 0 first.

```rust
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

        if state != ProgramState::Failed.as_str()
            || code.as_deref() != Some("daemon_restart")
        {
            return Err(StoreError::NotResumable(format!(
                "program {id} state={state} code={code:?}"
            )));
        }

        let new_cursor = if from_start { 0 } else { u32::try_from(cursor).unwrap_or(0) };

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
        Ok(ResumeOutcome { from_step: new_cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use vcli_core::state::ProgramState;
    use crate::store::NewProgram;

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
        s.set_last_error(id, "wait_for_timeout", "timed out").unwrap();
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib resume`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/resume.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: resume_program (failed(daemon_restart) → running)"
```

---

## Task 10: Events table — `append_event` + `stream_events(since)`

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/events.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module + re-export**

In `lib.rs`:

```rust
pub mod events;

pub use events::StoredEvent;
```

- [ ] **Step 2: Write `events.rs`**

The spec says: "events table holds **durable terminal events only**" and Decision 1.7 says "Terminal events (`program.completed`, `program.failed`) also persist in SQLite `events` table". We accept any `EventData` the caller hands us — enforcement of "terminal only" belongs in the runtime.

```rust
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
    /// Surfaces SQLite + serde errors.
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
    /// Surfaces SQLite errors.
    pub fn stream_events(&self, since: i64, limit: u32) -> StoreResult<Vec<StoredEvent>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, program_id, type, data_json, at
             FROM events
             WHERE id > ?1
             ORDER BY id ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![since, limit as i64], |r| {
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
    use tempfile::tempdir;
    use vcli_core::state::ProgramState;
    use crate::store::NewProgram;

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
            data: EventData::ProgramCompleted { program_id: id, emit: None },
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
            s.append_event(id, &Event {
                at: i,
                data: EventData::ProgramStateChanged {
                    program_id: id,
                    from: ProgramState::Waiting,
                    to: ProgramState::Running,
                    reason: format!("t{i}"),
                },
            }).unwrap();
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
            s.append_event(id, &Event {
                at: 0,
                data: EventData::ProgramCompleted { program_id: id, emit: None },
            }).unwrap();
        }
        let rows = s.stream_events(0, 3).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn cascade_delete_removes_events() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = seed(&mut s);
        s.append_event(id, &Event {
            at: 0,
            data: EventData::ProgramCompleted { program_id: id, emit: None },
        }).unwrap();
        s.conn_mut().execute("DELETE FROM programs WHERE id = ?1", [id.to_string()]).unwrap();
        let rows = s.stream_events(0, 10).unwrap();
        assert!(rows.is_empty());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib events`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/events.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: append_event + stream_events(since, limit) with FK cascade"
```

---

## Task 11: Traces — flush-on-demand ring

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/traces.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module + re-export**

```rust
pub mod traces;

pub use traces::{TraceRecord, TraceKind};
```

- [ ] **Step 2: Write `traces.rs`**

The spec says traces are in-memory-only by default. Per the prompt, the store
exposes `flush_traces(records)` so the runtime can persist them at shutdown or
at an explicit boundary. The in-memory ring itself lives elsewhere (runtime);
this module is the persistence half only.

```rust
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
    /// Surfaces SQLite + serde errors.
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
    /// Surfaces SQLite errors.
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
                    "predicate_eval" => TraceKind::PredicateEval,
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
    use tempfile::tempdir;
    use vcli_core::state::ProgramState;
    use crate::store::NewProgram;

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
        s.flush_traces(&[rec.clone()]).unwrap();
        let back = s.read_traces(id).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].tick, 7);
        assert_eq!(back[0].kind, TraceKind::StateChange);
        assert_eq!(back[0].payload["to"], "running");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib traces`
Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-store/src/traces.rs crates/vcli-store/src/lib.rs
git commit -m "vcli-store: flush_traces + read_traces (persistent side of trace ring)"
```

---

## Task 12: `AssetStore` — `put_asset` + `get_asset` + dedup

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/assets.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module + re-export**

```rust
pub mod assets;

pub use assets::{AssetHash, PutAssetOutcome};
```

- [ ] **Step 2: Write `assets.rs`**

Content-addressed layout: `<data_root>/assets/sha256/<aa>/<bb>/<full_hex><.ext>`.
Writes use atomic temp-file-in-same-dir + rename. Dedup is fundamental: same
bytes → same hex → same path → same row in the `assets` index table.

```rust
//! Content-addressed asset store. Spec §Asset store (content-addressed).

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vcli_core::ids::ProgramId;

use crate::error::{StoreError, StoreResult};
use crate::paths::asset_blob_path;
use crate::store::Store;

/// SHA-256 digest of an asset, hex-encoded. Matches the `"sha256:<hex>"`
/// references inside a stored program's `source_json` (prefix stripped here).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetHash(String);

impl AssetHash {
    /// Raw 64-char lowercase hex string (no `sha256:` prefix).
    #[must_use]
    pub fn hex(&self) -> &str {
        &self.0
    }

    /// Wrap a precomputed hex hash. Caller is responsible for correctness
    /// (must be 64 lowercase hex chars).
    #[must_use]
    pub fn from_hex(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }

    /// Hash the given bytes.
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        let mut h = Sha256::new();
        h.update(bytes);
        let digest = h.finalize();
        Self(hex::encode(digest))
    }
}

impl std::fmt::Display for AssetHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Return value from `put_asset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutAssetOutcome {
    /// Content hash of the asset bytes.
    pub hash: AssetHash,
    /// True if the blob file was created by this call; false if it already existed.
    pub created: bool,
}

impl Store {
    /// Write `bytes` into the asset store if not already present and upsert the
    /// `assets` index row. Returns the hash and whether the file was newly
    /// created (deduplication signal).
    ///
    /// `extension` should be the lowercase file extension (no dot), e.g. `Some("png")`.
    /// Pass `None` for ext-less blobs.
    ///
    /// # Errors
    /// Surfaces SQLite + IO errors.
    pub fn put_asset(
        &mut self,
        bytes: &[u8],
        extension: Option<&str>,
        now_ms: i64,
    ) -> StoreResult<PutAssetOutcome> {
        let hash = AssetHash::of_bytes(bytes);
        let path = asset_blob_path(self.data_root(), hash.hex(), extension);

        let created = if path.exists() {
            false
        } else {
            let parent = path.parent().expect("blob path has a parent");
            fs::create_dir_all(parent).map_err(|e| StoreError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
            atomic_write(&path, bytes)?;
            true
        };

        // Upsert the `assets` row.
        self.conn_mut().execute(
            "INSERT INTO assets (hash, byte_len, extension, added_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(hash) DO NOTHING",
            rusqlite::params![
                hash.hex(),
                i64::try_from(bytes.len()).unwrap_or(i64::MAX),
                extension,
                now_ms,
            ],
        )?;
        Ok(PutAssetOutcome { hash, created })
    }

    /// Read the bytes of an asset by hash. Returns `None` if the blob is missing.
    ///
    /// # Errors
    /// Surfaces IO errors other than `NotFound`.
    pub fn get_asset(&self, hash: &AssetHash) -> StoreResult<Option<Vec<u8>>> {
        // Look up extension (so we construct the right path).
        let ext: Option<String> = self
            .conn()
            .query_row(
                "SELECT extension FROM assets WHERE hash = ?1",
                [hash.hex()],
                |r| r.get(0),
            )
            .unwrap_or(None);
        let path = asset_blob_path(self.data_root(), hash.hex(), ext.as_deref());
        match File::open(&path) {
            Ok(mut f) => {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(|e| StoreError::Io {
                    path: path.clone(),
                    source: e,
                })?;
                Ok(Some(buf))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StoreError::Io { path, source: e }),
        }
    }

    /// Link a program to an asset in `program_assets` (idempotent).
    ///
    /// # Errors
    /// Surfaces SQLite errors.
    pub fn link_program_asset(
        &mut self,
        program_id: ProgramId,
        hash: &AssetHash,
    ) -> StoreResult<()> {
        self.conn_mut().execute(
            "INSERT OR IGNORE INTO program_assets (program_id, asset_hash)
             VALUES (?1, ?2)",
            rusqlite::params![program_id.to_string(), hash.hex()],
        )?;
        Ok(())
    }

    /// Return the set of asset hashes referenced by any program row.
    ///
    /// # Errors
    /// Surfaces SQLite errors.
    pub fn referenced_asset_hashes(&self) -> StoreResult<Vec<AssetHash>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT DISTINCT asset_hash FROM program_assets")?;
        let rows = stmt.query_map([], |r| Ok(AssetHash::from_hex(r.get::<_, String>(0)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn atomic_write(path: &PathBuf, bytes: &[u8]) -> StoreResult<()> {
    let tmp_path = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp_path).map_err(|e| StoreError::Io {
            path: tmp_path.clone(),
            source: e,
        })?;
        f.write_all(bytes).map_err(|e| StoreError::Io {
            path: tmp_path.clone(),
            source: e,
        })?;
        f.sync_all().ok();
    }
    fs::rename(&tmp_path, path).map_err(|e| StoreError::Io {
        path: path.clone(),
        source: e,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn put_then_get_roundtrips_bytes() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let out = s.put_asset(b"hello", Some("txt"), 0).unwrap();
        assert!(out.created);
        let bytes = s.get_asset(&out.hash).unwrap().unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn asset_hash_of_bytes_is_deterministic() {
        let a = AssetHash::of_bytes(b"x");
        let b = AssetHash::of_bytes(b"x");
        assert_eq!(a, b);
        assert_ne!(a, AssetHash::of_bytes(b"y"));
    }

    #[test]
    fn put_asset_dedupes_same_bytes() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let a = s.put_asset(b"png-bytes-here", Some("png"), 0).unwrap();
        let b = s.put_asset(b"png-bytes-here", Some("png"), 10).unwrap();
        assert_eq!(a.hash, b.hash);
        assert!(a.created);
        assert!(!b.created, "second put must dedupe");
    }

    #[test]
    fn get_asset_missing_returns_none() {
        let d = tempdir().unwrap();
        let (s, _) = Store::open(d.path()).unwrap();
        let h = AssetHash::of_bytes(b"never-stored");
        assert_eq!(s.get_asset(&h).unwrap(), None);
    }

    #[test]
    fn link_program_asset_and_list() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        use vcli_core::state::ProgramState;
        use crate::store::NewProgram;
        let pid = ProgramId::new();
        s.insert_program(&NewProgram {
            id: pid, name: "p", source_json: "{}",
            state: ProgramState::Pending, submitted_at: 0, labels_json: "{}",
        }).unwrap();
        let h = s.put_asset(b"bytes", Some("png"), 0).unwrap().hash;
        s.link_program_asset(pid, &h).unwrap();
        s.link_program_asset(pid, &h).unwrap(); // idempotent
        let listed = s.referenced_asset_hashes().unwrap();
        assert_eq!(listed, vec![h]);
    }

    #[test]
    fn blob_lands_under_expected_sharded_path() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let out = s.put_asset(b"abc", Some("png"), 0).unwrap();
        let expected = asset_blob_path(d.path(), out.hash.hex(), Some("png"));
        assert!(expected.exists(), "missing blob at {expected:?}");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-store --lib assets`
Expected: 6 tests pass.

- [ ] **Step 4: Integration test for dedup**

Create `crates/vcli-store/tests/assets_dedup.rs`:

```rust
//! Integration: same bytes via different code paths yield the same hash
//! and the same single blob file on disk.

use tempfile::tempdir;
use vcli_store::{paths::asset_blob_path, Store};

#[test]
fn two_programs_sharing_an_asset_keep_one_blob() {
    let d = tempdir().unwrap();
    let (mut s, _) = Store::open(d.path()).unwrap();
    let a = s.put_asset(b"IMAGE-BYTES", Some("png"), 0).unwrap();
    let b = s.put_asset(b"IMAGE-BYTES", Some("png"), 0).unwrap();
    assert_eq!(a.hash, b.hash);
    // Single blob file exists on disk.
    let path = asset_blob_path(d.path(), a.hash.hex(), Some("png"));
    assert!(path.exists());
}
```

- [ ] **Step 5: Run integration**

Run: `cargo test -p vcli-store --test assets_dedup`
Expected: 1 test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/vcli-store/src/assets.rs crates/vcli-store/src/lib.rs crates/vcli-store/tests/assets_dedup.rs
git commit -m "vcli-store: content-addressed AssetStore (put/get/link/dedup)"
```

---

## Task 13: GC — programs and assets

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/src/gc.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-store/src/lib.rs`

- [ ] **Step 1: Append module + re-export**

```rust
pub mod gc;

pub use gc::{GcReport, RETENTION_DAYS};
```

- [ ] **Step 2: Write `gc.rs`**

Spec: "GC opportunistic: `vcli gc` is explicit; daemon triggers GC on startup if
last run was >7 days ago." We expose (1) program GC — delete terminal programs
older than N days, (2) asset GC — delete blob files and rows whose hash is no
longer referenced in `program_assets`. Never blocks the tick loop (these are
invoked explicitly; vcli-store is sync and decisions about when to run belong
to the runtime).

```rust
//! Garbage collection for terminal programs and unreferenced assets.
//! Spec §Persistence → Asset store (GC) + §Restart semantics step 9.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::assets::AssetHash;
use crate::error::{StoreError, StoreResult};
use crate::paths::{asset_blob_path, assets_root};
use crate::store::Store;

/// Default retention window for terminal programs and orphan assets.
pub const RETENTION_DAYS: u32 = 7;

/// What a GC pass did. All counts are 0-safe.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcReport {
    /// Number of `programs` rows deleted (with events/program_assets/traces
    /// cascading).
    pub programs_deleted: u32,
    /// Number of `assets` rows deleted.
    pub assets_deleted: u32,
    /// Number of blob files unlinked from disk.
    pub blobs_deleted: u32,
    /// Number of orphan blob files found (no `assets` row) and removed.
    pub orphan_blobs_deleted: u32,
}

impl Store {
    /// Delete terminal-state programs whose `finished_at` is older than
    /// `older_than_unix_ms`. FK cascades clean up events, program_assets,
    /// traces.
    ///
    /// # Errors
    /// Surfaces SQLite errors.
    pub fn gc_programs(&mut self, older_than_unix_ms: i64) -> StoreResult<u32> {
        let n = self.conn_mut().execute(
            "DELETE FROM programs
               WHERE state IN ('completed','failed','cancelled')
                 AND finished_at IS NOT NULL
                 AND finished_at < ?1",
            [older_than_unix_ms],
        )?;
        Ok(u32::try_from(n).unwrap_or(0))
    }

    /// Delete `assets` rows (and blobs on disk) that are no longer referenced
    /// by any program. Safe to run while the daemon is idle; never blocks the
    /// tick loop (spec: "Never blocks the tick loop").
    ///
    /// # Errors
    /// Surfaces SQLite + IO errors.
    pub fn gc_assets(&mut self) -> StoreResult<(u32, u32)> {
        // 1. Find unreferenced hashes.
        let unreferenced: Vec<(String, Option<String>)> = {
            let mut stmt = self.conn().prepare(
                "SELECT hash, extension FROM assets
                 WHERE hash NOT IN (SELECT DISTINCT asset_hash FROM program_assets)",
            )?;
            let rows = stmt.query_map([], |r| {
                let h: String = r.get(0)?;
                let e: Option<String> = r.get(1)?;
                Ok((h, e))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        // 2. Delete blobs.
        let mut blobs_deleted = 0u32;
        for (hash, ext) in &unreferenced {
            let path = asset_blob_path(self.data_root(), hash, ext.as_deref());
            if path.exists() {
                fs::remove_file(&path).map_err(|e| StoreError::Io {
                    path: path.clone(),
                    source: e,
                })?;
                blobs_deleted += 1;
            }
        }
        // 3. Delete rows.
        let assets_deleted = {
            let tx = self.conn_mut().transaction()?;
            let mut n = 0u32;
            {
                let mut stmt =
                    tx.prepare("DELETE FROM assets WHERE hash = ?1")?;
                for (hash, _) in &unreferenced {
                    n += u32::try_from(stmt.execute([hash])?).unwrap_or(0);
                }
            }
            tx.commit()?;
            n
        };
        Ok((assets_deleted, blobs_deleted))
    }

    /// Find orphan blob files (on disk but not in the `assets` table) and
    /// remove them. Used by the spec's "daemon triggers GC on startup if last
    /// run was >7 days ago" behavior; also useful post-crash.
    ///
    /// # Errors
    /// Surfaces IO errors.
    pub fn gc_orphan_blobs(&self) -> StoreResult<u32> {
        let known: HashSet<String> = {
            let mut stmt = self.conn().prepare("SELECT hash FROM assets")?;
            stmt.query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()?
        };
        let root = assets_root(self.data_root());
        if !root.exists() {
            return Ok(0);
        }
        let mut deleted = 0u32;
        walk_files(&root, &mut |path: &PathBuf| {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                // Strip any ".tmp" leftover too.
                if !known.contains(stem) && !stem.ends_with("tmp") {
                    let _ = fs::remove_file(path);
                    deleted += 1;
                }
            }
            Ok(())
        })?;
        Ok(deleted)
    }

    /// Full GC sweep. Convenience for `vcli gc` and startup.
    ///
    /// # Errors
    /// Surfaces SQLite + IO errors.
    pub fn gc_all(&mut self, older_than_unix_ms: i64) -> StoreResult<GcReport> {
        let programs_deleted = self.gc_programs(older_than_unix_ms)?;
        let (assets_deleted, blobs_deleted) = self.gc_assets()?;
        let orphan_blobs_deleted = self.gc_orphan_blobs()?;
        Ok(GcReport {
            programs_deleted,
            assets_deleted,
            blobs_deleted,
            orphan_blobs_deleted,
        })
    }

    /// Convenience: list orphan hashes (on disk but not in `assets`).
    /// Used by the spec's "log orphan count" restart step.
    ///
    /// # Errors
    /// Surfaces IO errors.
    pub fn list_orphan_blob_names(&self) -> StoreResult<Vec<String>> {
        let known: HashSet<String> = {
            let mut stmt = self.conn().prepare("SELECT hash FROM assets")?;
            stmt.query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()?
        };
        let root = assets_root(self.data_root());
        if !root.exists() {
            return Ok(vec![]);
        }
        let mut orphans = Vec::new();
        walk_files(&root, &mut |path: &PathBuf| {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if !known.contains(stem) && !stem.ends_with("tmp") {
                    orphans.push(stem.to_string());
                }
            }
            Ok(())
        })?;
        Ok(orphans)
    }
}

fn walk_files(
    dir: &std::path::Path,
    cb: &mut dyn FnMut(&PathBuf) -> StoreResult<()>,
) -> StoreResult<()> {
    for entry in fs::read_dir(dir).map_err(|e| StoreError::Io {
        path: dir.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| StoreError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        let ft = entry.file_type().map_err(|e| StoreError::Io {
            path: path.clone(),
            source: e,
        })?;
        if ft.is_dir() {
            walk_files(&path, cb)?;
        } else if ft.is_file() {
            cb(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use vcli_core::ids::ProgramId;
    use vcli_core::state::ProgramState;
    use crate::store::NewProgram;

    fn seed_terminal(s: &mut Store, finished_at: i64) -> ProgramId {
        let id = ProgramId::new();
        s.insert_program(&NewProgram {
            id, name: "x", source_json: "{}",
            state: ProgramState::Pending, submitted_at: 0, labels_json: "{}",
        }).unwrap();
        s.update_state(id, ProgramState::Completed, finished_at).unwrap();
        id
    }

    #[test]
    fn gc_programs_keeps_recent_and_prunes_old() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let old = seed_terminal(&mut s, 1_000);
        let recent = seed_terminal(&mut s, 10_000);
        let n = s.gc_programs(5_000).unwrap();
        assert_eq!(n, 1);
        assert!(s.get_program(old).is_err());
        assert!(s.get_program(recent).is_ok());
    }

    #[test]
    fn gc_programs_does_not_touch_active_rows() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = ProgramId::new();
        s.insert_program(&NewProgram {
            id, name: "x", source_json: "{}",
            state: ProgramState::Pending, submitted_at: 0, labels_json: "{}",
        }).unwrap();
        s.update_state(id, ProgramState::Running, 100).unwrap();
        let n = s.gc_programs(i64::MAX).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn gc_assets_removes_unreferenced_only() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let id = ProgramId::new();
        s.insert_program(&NewProgram {
            id, name: "x", source_json: "{}",
            state: ProgramState::Pending, submitted_at: 0, labels_json: "{}",
        }).unwrap();
        let keep = s.put_asset(b"A", Some("png"), 0).unwrap().hash;
        let drop = s.put_asset(b"B", Some("png"), 0).unwrap().hash;
        s.link_program_asset(id, &keep).unwrap();
        let (rows, blobs) = s.gc_assets().unwrap();
        assert_eq!(rows, 1);
        assert_eq!(blobs, 1);
        assert!(s.get_asset(&keep).unwrap().is_some());
        assert!(s.get_asset(&drop).unwrap().is_none());
    }

    #[test]
    fn gc_all_reports_all_counts() {
        let d = tempdir().unwrap();
        let (mut s, _) = Store::open(d.path()).unwrap();
        let _ = seed_terminal(&mut s, 1_000);
        let _ = s.put_asset(b"X", Some("png"), 0).unwrap();
        let report = s.gc_all(5_000).unwrap();
        assert_eq!(report.programs_deleted, 1);
        assert_eq!(report.assets_deleted, 1);
        assert_eq!(report.blobs_deleted, 1);
    }

    #[test]
    fn gc_orphan_blobs_removes_files_without_rows() {
        let d = tempdir().unwrap();
        let (s, _) = Store::open(d.path()).unwrap();
        // Drop a file manually into the assets dir.
        use std::fs;
        let orphan = asset_blob_path(
            d.path(),
            "ab12deadbeefbabecafefeedface0000000000000000000000000000000000aa",
            Some("png"),
        );
        fs::create_dir_all(orphan.parent().unwrap()).unwrap();
        fs::write(&orphan, b"orphan").unwrap();
        assert!(orphan.exists());
        let n = s.gc_orphan_blobs().unwrap();
        assert_eq!(n, 1);
        assert!(!orphan.exists());
    }
}
```

- [ ] **Step 3: Run lib tests**

Run: `cargo test -p vcli-store --lib gc`
Expected: 5 tests pass.

- [ ] **Step 4: Integration test for 7-day retention**

Create `crates/vcli-store/tests/gc.rs`:

```rust
//! Integration: the 7-day retention boundary keeps recent terminal programs
//! and prunes older ones.

use tempfile::tempdir;
use vcli_core::ids::ProgramId;
use vcli_core::state::ProgramState;
use vcli_store::{NewProgram, Store, RETENTION_DAYS};

const MS_PER_DAY: i64 = 86_400_000;

#[test]
fn seven_day_retention_boundary() {
    let d = tempdir().unwrap();
    let (mut s, _) = Store::open(d.path()).unwrap();
    let now = 1_700_000_000_000_i64;
    let cutoff = now - i64::from(RETENTION_DAYS) * MS_PER_DAY;

    let a = ProgramId::new();
    let b = ProgramId::new();
    for (id, finished) in [(a, cutoff - 1), (b, cutoff + 1)] {
        s.insert_program(&NewProgram {
            id, name: "x", source_json: "{}",
            state: ProgramState::Pending, submitted_at: 0, labels_json: "{}",
        }).unwrap();
        s.update_state(id, ProgramState::Completed, finished).unwrap();
    }

    let n = s.gc_programs(cutoff).unwrap();
    assert_eq!(n, 1);
    assert!(s.get_program(a).is_err());
    assert!(s.get_program(b).is_ok());
}
```

- [ ] **Step 5: Run integration**

Run: `cargo test -p vcli-store --test gc`
Expected: 1 test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/vcli-store/src/gc.rs crates/vcli-store/src/lib.rs crates/vcli-store/tests/gc.rs
git commit -m "vcli-store: 7-day GC for terminal programs + unreferenced assets + orphan blobs"
```

---

## Task 14: Concurrent-readers WAL integration test

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-store/tests/wal_concurrent.rs`

- [ ] **Step 1: Write the test**

```rust
//! Integration: SQLite WAL mode permits concurrent readers during writes.
//! Opens one writer + two readers in separate threads and asserts no
//! `SQLITE_BUSY` failures within the 5s busy timeout.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rusqlite::Connection;
use tempfile::tempdir;
use vcli_core::ids::ProgramId;
use vcli_core::state::ProgramState;
use vcli_store::{paths::db_path, pragmas::apply_pragmas, NewProgram, Store};

#[test]
fn two_readers_coexist_with_one_writer() {
    let d = tempdir().unwrap();
    let (mut s, _) = Store::open(d.path()).unwrap();
    // Seed enough rows for readers to iterate.
    for i in 0..50 {
        let id = ProgramId::new();
        s.insert_program(&NewProgram {
            id,
            name: "seed",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: i,
            labels_json: "{}",
        })
        .unwrap();
    }

    let path = Arc::new(db_path(d.path()));

    // Reader thread helper.
    let mk_reader = |iters: usize| {
        let path = path.clone();
        thread::spawn(move || {
            let c = Connection::open(&*path).unwrap();
            apply_pragmas(&c).unwrap();
            for _ in 0..iters {
                let n: i64 = c
                    .query_row("SELECT COUNT(*) FROM programs", [], |r| r.get(0))
                    .unwrap();
                assert!(n >= 50);
                thread::sleep(Duration::from_millis(2));
            }
        })
    };

    let r1 = mk_reader(20);
    let r2 = mk_reader(20);

    // Writer does inserts concurrently.
    let w = {
        let path = path.clone();
        thread::spawn(move || {
            let c = Connection::open(&*path).unwrap();
            apply_pragmas(&c).unwrap();
            for i in 0..20 {
                c.execute(
                    "INSERT INTO programs
                       (id, name, source_json, state, submitted_at, labels_json, body_cursor)
                     VALUES (?1, 'w', '{}', 'pending', ?2, '{}', 0)",
                    rusqlite::params![ProgramId::new().to_string(), 1000 + i],
                )
                .unwrap();
            }
        })
    };

    r1.join().unwrap();
    r2.join().unwrap();
    w.join().unwrap();

    // Final count = 50 seed + 20 inserts = 70.
    let (s, _) = Store::open(d.path()).unwrap();
    let n: i64 = s
        .conn()
        .query_row("SELECT COUNT(*) FROM programs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 70);
}
```

Note: `Store::conn()` is `pub(crate)` in Task 6 — expose it as `pub` (read-only) for this test. Modify `store.rs`:

```rust
-    pub(crate) fn conn(&self) -> &Connection {
+    /// Read-only access to the underlying connection (integration tests + gc).
+    #[must_use]
+    pub fn conn(&self) -> &Connection {
```

Similarly `pragmas::apply_pragmas` and `paths::db_path` need to be `pub` (they already are).

- [ ] **Step 2: Run**

Run: `cargo test -p vcli-store --test wal_concurrent`
Expected: 1 test passes.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-store/src/store.rs crates/vcli-store/tests/wal_concurrent.rs
git commit -m "vcli-store: WAL concurrent-readers integration test"
```

---

## Task 15: Full-crate verification

**Files:** (none new)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test -p vcli-store`
Expected: all tests pass — roughly 5 (error) + 5 (paths) + 3 (pragmas) + 5 (migrations) + 7 (store) + 4 (resume) + 4 (events) + 2 (traces) + 6 (assets) + 5 (gc) unit + 2 restart + 1 assets_dedup + 1 gc + 1 wal = ~51.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p vcli-store --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 3: Run rustfmt check**

Run: `cargo fmt --all -- --check`
Expected: no diff.

- [ ] **Step 4: Build docs**

Run: `cargo doc -p vcli-store --no-deps`
Expected: builds with no `missing_docs` warnings.

- [ ] **Step 5: Run full workspace tests (ensure vcli-core still passes)**

Run: `cargo test --workspace --locked`
Expected: vcli-core + vcli-store all pass.

- [ ] **Step 6: Tag the milestone**

```bash
git tag lane-f-vcli-store-complete -m "vcli-store complete — persistence + asset store + GC + restart recovery"
```

---

## Self-review

- **Spec coverage**:
  - `programs` + `program_assets` + `events` + `schema_version` tables: Task 5 (schema matches spec §SQLite schema v0 verbatim).
  - `assets` index + `traces` table: Task 5 (prompt-required extensions).
  - Content-addressed blob layout `assets/sha256/<aa>/<bb>/<hex>.<ext>`: Tasks 3 + 12.
  - WAL + PRAGMAs (Decision 4.4): Task 4 + applied by every `Store::open`.
  - Restart recovery `running → failed(daemon_restart)` with cursor preserved (spec §Restart semantics step 4 + Phase A step 4): Tasks 6 + 8.
  - `resume_program` (spec §Resume + Decision 2.4 + Decision C): Task 9 (eligibility per Decision C stays in the runtime — documented).
  - Events persistence of terminal events (Decision 1.7) + stream cursor: Task 10.
  - Trace ring flush-on-demand: Task 11 (in-memory ring lives in runtime, persistence half here).
  - `put_asset`/`get_asset` dedup: Task 12 + `assets_dedup.rs`.
  - 7-day GC: Task 13 + `tests/gc.rs` using `RETENTION_DAYS`.
  - Concurrent readers under WAL: Task 14.
- **Style match**: Bite-sized 2-5 minute steps, failing-test-first where meaningful, `cargo test -p vcli-store --lib <module>` after each task, commit messages `vcli-store: …`, file paths absolute-from-repo-root.
- **Dependencies**: only `vcli-core` (as required) + documented tech-stack deps. No `vcli-runtime`, no IPC, no perception, no capture, no input imports.
- **No placeholders**: every schema is full SQL, every test has real assertions, every rusqlite call names columns.
- **Ambiguities flagged in the final report**, not silently resolved.

---

**Report back:**

1. **Path written:** `docs/superpowers/plans/2026-04-16-vcli-store.md` — NOTE: I was run in read-only planning mode with no file-editing tools available. I did NOT create the file. The complete plan is delivered inline as the assistant message above; the user / parent agent should persist it to that path. Every step is ready to paste verbatim into the plan file.

2. **Total task count:** 15 tasks (Task 1 wiring + Task 2 error + Task 3 paths + Task 4 pragmas + Task 5 migrations + Task 6 open/recovery + Task 7 programs CRUD + Task 8 restart-recovery integration + Task 9 resume + Task 10 events + Task 11 traces + Task 12 assets + Task 13 GC + Task 14 WAL concurrent integration + Task 15 full-crate verification).

3. **rusqlite vs sqlx choice — rusqlite, because:**
   - **Sync fits the scheduler.** The daemon's tick loop is single-threaded and sync (spec §Runtime & scheduler threading model: "tokio runtime in the daemon never contaminates the scheduler's logic"). `vcli-store` is called from that sync thread — an async client would force every call site to bridge a runtime.
   - **Smaller footprint.** `rusqlite` with `bundled` pins SQLite version deterministically, compiles in ~30s, and doesn't pull in the `sqlx` proc-macro machinery or a compile-time query-checker DB connection — both overkill for hand-written migrations and a dozen queries.
   - **Existing ecosystem fit.** `rusqlite::Connection` is trivially embeddable behind the `Store` struct; `sqlx::SqlitePool` wants an async context that doesn't exist here.
   - `sqlx` would only win if we needed a shared async connection pool across many tokio tasks — we don't.

4. **Spec ambiguities called out:**
   - **"resume_cursor" vs spec `body_cursor`.** The prompt instructs me to store a `resume_cursor` column; the spec schema names it `body_cursor` and treats it as the single source of truth for resumption. I aligned with the spec: the `programs.body_cursor` column IS the resume cursor. `resume_program` returns a `ResumeOutcome { from_step }` so callers never need to know the column name. **Recommend confirming this alignment with other lanes.**
   - **Traces persistence.** Spec says "per-tick traces (in-memory ring)… **Not in the DB**". The prompt requires a `traces` table persisted at task boundaries / shutdown. I took the overlap interpretation: provide a `traces` table + `flush_traces` / `read_traces` as an **opt-in shutdown-or-boundary flush** — the in-memory ring still lives in `vcli-runtime` and the hot path never hits SQLite for traces. Runtime decides when (if ever) to flush.
   - **Asset path extension handling.** Spec example shows `assets/sha256/ab/cd/abcd…ef.png` (keeps the extension). The DSL rewrites `image: "path.png"` → `image: "sha256:<hash>"` — no extension in the reference. I chose to store extension as an optional `assets.extension` column and make it part of the filename purely for human readability; `get_asset(hash)` works whether or not an extension is known (looks it up via the index). **Recommend confirming this is OK with vcli-dsl lane B** (they're the ones ingesting the asset at submit time).
   - **Who performs the restart-recovery event emission.** I have `Store::open` return `Vec<RecoveredProgram>` so the runtime can emit `program.state_changed` + `program.failed` events over the event bus. The store itself does NOT write events rows during recovery (avoids coupling to event-bus semantics and avoids double-emission if the runtime re-emits). **Runtime lane must own emission using this return value.**
   - **`on_schedule` trigger & `WallClock`.** Decision D removes `on_schedule` from v0, so there's no need for wall-clock timestamps beyond `UnixMs` already supplied by `vcli-core::Clock`. No action in this lane; noted.
