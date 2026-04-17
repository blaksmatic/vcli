//! Smoke test: the YT ad skipper fixture canonicalizes and hashes stably.

use vcli_core::canonicalize;
use vcli_core::predicate_hash;
use vcli_core::Program;

const YT_FIXTURE: &str = include_str!("../../../fixtures/yt_ad_skipper.json");

#[test]
fn fixture_canonical_bytes_stable_across_parse_reserialize() {
    let p: Program = serde_json::from_str(YT_FIXTURE).unwrap();
    let reser = serde_json::to_value(&p).unwrap();
    let v1: serde_json::Value = serde_json::from_str(YT_FIXTURE).unwrap();
    assert_eq!(canonicalize(&v1).unwrap(), canonicalize(&reser).unwrap());
}

#[test]
fn fixture_hash_is_stable() {
    let v1: serde_json::Value = serde_json::from_str(YT_FIXTURE).unwrap();
    let v2: serde_json::Value = serde_json::from_str(YT_FIXTURE).unwrap();
    assert_eq!(predicate_hash(&v1).unwrap(), predicate_hash(&v2).unwrap());
}
