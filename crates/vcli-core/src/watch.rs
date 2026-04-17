//! Reactive watches — `when → do` with a lifetime.
//!
//! `when` can be either a predicate name (referencing a named entry in the
//! program's `predicates` map) or an inline anonymous predicate. The DSL
//! validator checks name references; inline predicates are validated in
//! place.

use serde::{Deserialize, Serialize};

use crate::predicate::PredicateKind;
use crate::step::Step;

/// Named-or-inline predicate reference for `watch.when`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WatchWhen {
    /// Reference by name. The name must exist in the enclosing program's
    /// `predicates` map (validated in `vcli-dsl`).
    ByName(String),
    /// Inline anonymous predicate.
    Inline(Box<PredicateKind>),
}

/// How long a watch stays active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Lifetime {
    /// Fires exactly once when `when` transitions false→true, then is removed.
    OneShot,
    /// Fires every false→true transition, respecting `throttle_ms`.
    Persistent,
    /// Persistent until the named predicate becomes truthy.
    UntilPredicate {
        /// Predicate name that, when truthy, removes this watch.
        name: String,
    },
    /// Persistent until N milliseconds after the program started `running`.
    TimeoutMs {
        /// Duration in ms from `running` entry.
        ms: u32,
    },
}

/// A reactive rule on a program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watch {
    /// Truth condition.
    pub when: WatchWhen,
    /// Steps to run on each fire. Only input Steps are valid here (validator
    /// rejects `wait_for` / `assert` / `sleep_ms` inside watches).
    #[serde(rename = "do")]
    pub steps: Vec<Step>,
    /// Minimum ms between fires. Defaults to 0 (no throttle).
    #[serde(default)]
    pub throttle_ms: u32,
    /// Persistence policy. Defaults to `persistent`.
    #[serde(default = "default_lifetime")]
    pub lifetime: Lifetime,
}

fn default_lifetime() -> Lifetime {
    Lifetime::Persistent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Button;
    use crate::step::Target;

    #[test]
    fn by_name_watch_roundtrip() {
        let w = Watch {
            when: WatchWhen::ByName("skip_visible".into()),
            steps: vec![Step::Click {
                at: Target::Expression("$skip_visible.match.center".into()),
                button: Button::Left,
            }],
            throttle_ms: 500,
            lifetime: Lifetime::Persistent,
        };
        let j = serde_json::to_string(&w).unwrap();
        let back: Watch = serde_json::from_str(&j).unwrap();
        assert_eq!(back, w);
    }

    #[test]
    fn inline_predicate_watch_parses() {
        let j = r#"{
            "when": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":10},
            "do": [{"kind":"move","at":{"x":0,"y":0}}]
        }"#;
        let w: Watch = serde_json::from_str(j).unwrap();
        matches!(w.when, WatchWhen::Inline(_));
        assert_eq!(w.throttle_ms, 0);
        assert_eq!(w.lifetime, Lifetime::Persistent);
    }

    #[test]
    fn lifetime_variants_roundtrip() {
        for l in [
            Lifetime::OneShot,
            Lifetime::Persistent,
            Lifetime::UntilPredicate { name: "done".into() },
            Lifetime::TimeoutMs { ms: 30_000 },
        ] {
            let j = serde_json::to_string(&l).unwrap();
            let back: Lifetime = serde_json::from_str(&j).unwrap();
            assert_eq!(back, l);
        }
    }
}
