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
        let p = asset_blob_path(Path::new("/r"), "deadbeef", None);
        assert_eq!(p, PathBuf::from("/r/assets/sha256/de/ad/deadbeef"));
    }

    #[test]
    #[should_panic(expected = "hex hash too short")]
    fn blob_path_panics_on_short_hash() {
        let _ = asset_blob_path(Path::new("/r"), "ab", None);
    }
}
