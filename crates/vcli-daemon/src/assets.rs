//! Daemon-side asset materialization for submitted programs.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;
use vcli_core::{ErrorCode, ErrorPayload, PredicateKind, Program, ProgramId, WatchWhen};
use vcli_store::{AssetHash, Store, StoreError};

/// Materialized template asset bytes, keyed by raw hash hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedAssets {
    /// Bytes keyed by raw SHA-256 hex digest.
    pub bytes: BTreeMap<String, Vec<u8>>,
    /// Hashes referenced by the rewritten program. The handler links these to
    /// `program_assets` after inserting the program row.
    pub hashes: Vec<AssetHash>,
}

impl MaterializedAssets {
    fn empty() -> Self {
        Self {
            bytes: BTreeMap::new(),
            hashes: Vec::new(),
        }
    }

    fn insert(&mut self, hash: &AssetHash, bytes: Vec<u8>) {
        if !self.hashes.iter().any(|h| h == hash) {
            self.hashes.push(hash.clone());
        }
        self.bytes.entry(hash.hex().to_string()).or_insert(bytes);
    }
}

/// Asset ingestion failure during submit/resume.
#[derive(Debug, Error)]
pub enum AssetMaterializeError {
    /// A relative path cannot be resolved without submit metadata from the CLI.
    #[error("relative template asset {asset:?} requires submit base_dir")]
    MissingBaseDir {
        /// Asset reference from the program.
        asset: String,
    },
    /// Asset file could not be read.
    #[error("read template asset {path:?}: {source}")]
    Io {
        /// Resolved filesystem path.
        path: PathBuf,
        /// Underlying IO cause.
        #[source]
        source: io::Error,
    },
    /// A `sha256:<hex>` reference could not be loaded from the store.
    #[error("template asset sha256:{hash} is not materialized")]
    UnknownAsset {
        /// Raw hash hex.
        hash: String,
    },
    /// Store write/read failed.
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

impl AssetMaterializeError {
    /// IPC error code for this failure.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::MissingBaseDir { .. } | Self::Io { .. } | Self::UnknownAsset { .. } => {
                ErrorCode::InvalidProgram
            }
            Self::Store(_) => ErrorCode::Internal,
        }
    }

    /// Convert to a stable IPC payload.
    #[must_use]
    pub fn to_payload(&self) -> ErrorPayload {
        ErrorPayload::simple(self.code(), self.to_string())
    }
}

/// Resolve non-`sha256:` template image refs, store them in the CAS, and
/// rewrite the program in place to `sha256:<hash>` refs.
///
/// # Errors
/// Returns `AssetMaterializeError` for missing base dirs, missing files,
/// unknown pre-materialized hashes, or store errors.
pub fn materialize_template_assets(
    store: &mut Store,
    _program_id: ProgramId,
    program: &mut Program,
    base_dir: Option<&Path>,
    now_ms: i64,
) -> Result<MaterializedAssets, AssetMaterializeError> {
    let mut out = MaterializedAssets::empty();
    for predicate in program.predicates.values_mut() {
        materialize_predicate(store, predicate, base_dir, now_ms, &mut out)?;
    }
    for watch in &mut program.watches {
        if let WatchWhen::Inline(predicate) = &mut watch.when {
            materialize_predicate(store, predicate, base_dir, now_ms, &mut out)?;
        }
    }
    Ok(out)
}

fn materialize_predicate(
    store: &mut Store,
    predicate: &mut PredicateKind,
    base_dir: Option<&Path>,
    now_ms: i64,
    out: &mut MaterializedAssets,
) -> Result<(), AssetMaterializeError> {
    if let PredicateKind::Template { image, .. } = predicate {
        materialize_image_ref(store, image, base_dir, now_ms, out)?;
    }
    Ok(())
}

fn materialize_image_ref(
    store: &mut Store,
    image: &mut String,
    base_dir: Option<&Path>,
    now_ms: i64,
    out: &mut MaterializedAssets,
) -> Result<(), AssetMaterializeError> {
    if let Some(hex) = image.strip_prefix("sha256:") {
        let hash = AssetHash::from_hex(hex);
        let bytes = store
            .get_asset(&hash)?
            .ok_or_else(|| AssetMaterializeError::UnknownAsset {
                hash: hash.hex().to_string(),
            })?;
        out.insert(&hash, bytes);
        return Ok(());
    }

    let asset_ref = image.clone();
    let raw_path = Path::new(&asset_ref);
    let resolved = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        base_dir
            .ok_or_else(|| AssetMaterializeError::MissingBaseDir {
                asset: asset_ref.clone(),
            })?
            .join(raw_path)
    };
    let bytes = fs::read(&resolved).map_err(|source| AssetMaterializeError::Io {
        path: resolved.clone(),
        source,
    })?;
    let extension = resolved
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase);
    let stored = store.put_asset(&bytes, extension.as_deref(), now_ms)?;
    *image = format!("sha256:{}", stored.hash.hex());
    out.insert(&stored.hash, bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;
    use vcli_core::predicate::{Confidence, PredicateKind};
    use vcli_core::program::DslVersion;
    use vcli_core::region::Region;
    use vcli_core::trigger::Trigger;
    use vcli_core::{Program, ProgramId};
    use vcli_store::Store;

    use super::materialize_template_assets;

    fn template_program(image: &str) -> Program {
        let mut predicates = BTreeMap::new();
        predicates.insert(
            "skip".into(),
            PredicateKind::Template {
                image: image.into(),
                confidence: Confidence(0.9),
                region: Region::Absolute {
                    rect: vcli_core::geom::Rect {
                        x: 0,
                        y: 0,
                        w: 10,
                        h: 10,
                    },
                },
                throttle_ms: 200,
            },
        );
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: "p".into(),
            id: None,
            trigger: Trigger::OnSubmit,
            predicates,
            watches: vec![],
            body: vec![],
            on_complete: None,
            on_fail: None,
            timeout_ms: None,
            labels: BTreeMap::new(),
            priority: vcli_core::Priority::default(),
        }
    }

    #[test]
    fn relative_template_is_ingested_and_rewritten() {
        let d = tempdir().unwrap();
        let asset_dir = d.path().join("program");
        std::fs::create_dir(&asset_dir).unwrap();
        std::fs::write(asset_dir.join("skip.png"), b"PNG-BYTES").unwrap();
        let (mut store, _) = Store::open(d.path().join("data")).unwrap();
        let pid = ProgramId::new();
        let mut program = template_program("skip.png");

        let out = materialize_template_assets(&mut store, pid, &mut program, Some(&asset_dir), 7)
            .unwrap();

        let PredicateKind::Template { image, .. } = &program.predicates["skip"] else {
            panic!("wrong predicate");
        };
        assert!(image.starts_with("sha256:"), "{image}");
        let hash = image.strip_prefix("sha256:").unwrap();
        assert_eq!(
            out.bytes.get(hash).map(Vec::as_slice),
            Some(&b"PNG-BYTES"[..])
        );
        assert_eq!(out.hashes.len(), 1);
        assert_eq!(out.hashes[0].hex(), hash);
    }

    #[test]
    fn missing_relative_base_dir_is_invalid_program() {
        let d = tempdir().unwrap();
        let (mut store, _) = Store::open(d.path().join("data")).unwrap();
        let pid = ProgramId::new();
        let mut program = template_program("skip.png");

        let err = materialize_template_assets(&mut store, pid, &mut program, None, 7).unwrap_err();

        assert_eq!(err.code(), vcli_core::ErrorCode::InvalidProgram);
        assert!(err.to_string().contains("base_dir"), "{err}");
    }
}
