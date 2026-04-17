//! `Program` — the top-level DSL document. Matches spec §DSL → Program shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::ProgramId;
use crate::predicate::PredicateKind;
use crate::step::Step;
use crate::trigger::Trigger;
use crate::watch::Watch;

/// DSL major version the daemon understands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DslVersion(pub String);

impl DslVersion {
    /// Current v0 DSL version.
    pub const V0_1: &'static str = "0.1";

    /// Major digit (everything before the first `.`).
    #[must_use]
    pub fn major(&self) -> &str {
        self.0.split('.').next().unwrap_or("")
    }
}

/// Priority for action arbitration (Decision 1.5). Higher wins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Priority(pub i32);

/// Event emitter for program completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnComplete {
    /// Custom event name to emit alongside the system `program.completed` event.
    pub emit: String,
}

/// Event emitter for program failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnFail {
    /// Custom event name to emit alongside the system `program.failed` event.
    pub emit: String,
}

/// Top-level program document.
///
/// Labels and predicate names use `BTreeMap` for deterministic canonical JSON
/// output — see Decision 1.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// DSL version string.
    pub version: DslVersion,
    /// Human label (not unique).
    pub name: String,
    /// Optional client-supplied id. Daemon assigns a fresh UUID when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ProgramId>,
    /// Program start trigger.
    pub trigger: Trigger,
    /// Named predicates.
    #[serde(default)]
    pub predicates: BTreeMap<String, PredicateKind>,
    /// Reactive rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watches: Vec<Watch>,
    /// Sequential body. Always serialized (even when empty) so that
    /// canonical JSON is stable across parse → re-serialize round-trips.
    #[serde(default)]
    pub body: Vec<Step>,
    /// Optional completion emitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_complete: Option<OnComplete>,
    /// Optional failure emitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_fail: Option<OnFail>,
    /// Program-level timeout. `None` = no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
    /// Free-form tags for filtering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// Priority for arbitration tiebreak (Decision 1.5).
    #[serde(default, skip_serializing_if = "is_default_priority")]
    pub priority: Priority,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_default_priority(p: &Priority) -> bool {
    p.0 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const YT_FIXTURE: &str = include_str!("../../../fixtures/yt_ad_skipper.json");

    #[test]
    fn dsl_version_major() {
        assert_eq!(DslVersion("0.1".into()).major(), "0");
        assert_eq!(DslVersion("1.2.3".into()).major(), "1");
    }

    #[test]
    fn yt_ad_skipper_fixture_parses() {
        let p: Program = serde_json::from_str(YT_FIXTURE).expect("fixture must parse");
        assert_eq!(p.name, "yt-ad-skipper");
        assert_eq!(p.version.0, "0.1");
        assert_eq!(p.predicates.len(), 1);
        assert!(p.predicates.contains_key("skip_visible"));
        assert_eq!(p.watches.len(), 1);
        assert!(p.body.is_empty());
        assert_eq!(p.on_complete.as_ref().unwrap().emit, "ad_skipped");
        assert_eq!(p.priority, Priority::default());
    }

    #[test]
    fn yt_ad_skipper_fixture_roundtrips() {
        let p: Program = serde_json::from_str(YT_FIXTURE).unwrap();
        let j = serde_json::to_string(&p).unwrap();
        let p2: Program = serde_json::from_str(&j).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn priority_default_is_omitted_from_serialization() {
        let p = Program {
            version: DslVersion("0.1".into()),
            name: "x".into(),
            id: None,
            trigger: Trigger::OnSubmit,
            predicates: BTreeMap::new(),
            watches: vec![],
            body: vec![],
            on_complete: None,
            on_fail: None,
            timeout_ms: None,
            labels: BTreeMap::new(),
            priority: Priority::default(),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(
            !j.contains("priority"),
            "default priority must not serialize: {j}"
        );
    }

    #[test]
    fn priority_nonzero_serializes() {
        let p = Program {
            version: DslVersion("0.1".into()),
            name: "x".into(),
            id: None,
            trigger: Trigger::OnSubmit,
            predicates: BTreeMap::new(),
            watches: vec![],
            body: vec![],
            on_complete: None,
            on_fail: None,
            timeout_ms: None,
            labels: BTreeMap::new(),
            priority: Priority(5),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""priority":5"#), "got {j}");
    }
}
