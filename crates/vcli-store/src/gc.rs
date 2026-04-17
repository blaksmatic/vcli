//! Garbage collection for terminal programs and unreferenced assets.
//! Spec §Persistence → Asset store (GC) + §Restart semantics step 9.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::error::{StoreError, StoreResult};
use crate::paths::{asset_blob_path, assets_root};
use crate::store::Store;

/// Default retention window for terminal programs and orphan assets.
pub const RETENTION_DAYS: u32 = 7;

/// What a GC pass did. All counts are 0-safe.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcReport {
    /// Number of `programs` rows deleted (with `events/program_assets/traces`
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
    /// `older_than_unix_ms`. FK cascades clean up events, `program_assets`,
    /// traces.
    ///
    /// # Errors
    /// Surfaces `SQLite` errors.
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
    /// Surfaces `SQLite` + IO errors.
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
                let mut stmt = tx.prepare("DELETE FROM assets WHERE hash = ?1")?;
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
    /// Surfaces `SQLite` + IO errors.
    pub fn gc_orphan_blobs(&self) -> StoreResult<u32> {
        let known: HashSet<String> = {
            let mut stmt = self.conn().prepare("SELECT hash FROM assets")?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()?;
            rows
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
    /// Surfaces `SQLite` + IO errors.
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
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()?;
            rows
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
    use crate::store::NewProgram;
    use std::fs;
    use tempfile::tempdir;
    use vcli_core::ids::ProgramId;
    use vcli_core::state::ProgramState;

    fn seed_terminal(s: &mut Store, finished_at: i64) -> ProgramId {
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
        s.update_state(id, ProgramState::Completed, finished_at)
            .unwrap();
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
            id,
            name: "x",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
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
            id,
            name: "x",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
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
