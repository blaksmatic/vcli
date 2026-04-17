//! End-to-end validation of the canonical demo fixture.

use vcli_core::{PredicateKind, Trigger, WatchWhen};
use vcli_dsl::{validate_str, Validated};

const YT_FIXTURE: &str = include_str!("../../../fixtures/yt_ad_skipper.json");

#[test]
fn yt_ad_skipper_validates_successfully() {
    let Validated { program, hashes } = validate_str(YT_FIXTURE).expect("fixture must validate");
    assert_eq!(program.name, "yt-ad-skipper");
    assert_eq!(program.version.0, "0.1");
    assert!(matches!(program.trigger, Trigger::OnSubmit));
    assert_eq!(program.predicates.len(), 1);
    assert!(matches!(
        program.predicates.get("skip_visible").unwrap(),
        PredicateKind::Template { .. }
    ));
    assert_eq!(program.watches.len(), 1);
    match &program.watches[0].when {
        WatchWhen::ByName(n) => assert_eq!(n, "skip_visible"),
        other => panic!("expected ByName: {other:?}"),
    }
    assert!(program.body.is_empty());
    assert_eq!(
        program.on_complete.as_ref().unwrap().emit,
        "ad_skipped"
    );

    assert_eq!(hashes.len(), 1);
    let h = &hashes["skip_visible"];
    assert_eq!(h.hex().len(), 64);
    assert!(h.hex().chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn yt_ad_skipper_hash_is_deterministic() {
    let a = validate_str(YT_FIXTURE).unwrap().hashes;
    let b = validate_str(YT_FIXTURE).unwrap().hashes;
    assert_eq!(a, b);
}

#[test]
fn mutating_confidence_changes_hash() {
    let mutated = YT_FIXTURE.replace(r#""confidence": 0.9"#, r#""confidence": 0.95"#);
    assert_ne!(mutated, YT_FIXTURE, "sanity: replacement must hit");
    let a = validate_str(YT_FIXTURE).unwrap().hashes;
    let b = validate_str(&mutated).unwrap().hashes;
    assert_ne!(a["skip_visible"], b["skip_visible"]);
}
