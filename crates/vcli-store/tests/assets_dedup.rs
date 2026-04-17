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
