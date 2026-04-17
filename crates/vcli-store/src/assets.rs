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
        use crate::store::NewProgram;
        use vcli_core::state::ProgramState;
        let pid = ProgramId::new();
        s.insert_program(&NewProgram {
            id: pid,
            name: "p",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
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
