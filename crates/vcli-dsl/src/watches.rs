//! Validate each `Watch`:
//! - `when`: `ByName` must resolve in `predicates`; `Inline` predicates are
//!   shape-checked (using the same rules as the named map, against the same
//!   name set for cross-references).
//! - `do`: via `validate_watch_steps`.
//! - `lifetime.until_predicate.name`: must resolve.

use std::collections::BTreeMap;

use vcli_core::{Lifetime, PredicateKind, Watch, WatchWhen};

use crate::error::{DslError, DslErrorKind};
use crate::hint::did_you_mean;
use crate::path::JsonPath;
use crate::predicates::validate_predicate_graph;
use crate::steps::validate_watch_steps;

/// Validate every watch on the program.
///
/// # Errors
///
/// Returns the first validation error found.
pub fn validate_watches(
    watches: &[Watch],
    predicates: &BTreeMap<String, PredicateKind>,
) -> Result<(), DslError> {
    for (i, w) in watches.iter().enumerate() {
        let at = JsonPath::root().key("watches").index(i);
        validate_watch_when(&w.when, predicates, &at.key("when"))?;
        validate_watch_steps(&w.steps, predicates, &at.key("do"))?;
        validate_lifetime(&w.lifetime, predicates, &at.key("lifetime"))?;
    }
    Ok(())
}

fn validate_watch_when(
    when: &WatchWhen,
    predicates: &BTreeMap<String, PredicateKind>,
    at: &JsonPath,
) -> Result<(), DslError> {
    match when {
        WatchWhen::ByName(name) => {
            if !predicates.contains_key(name) {
                let names: Vec<&str> = predicates.keys().map(String::as_str).collect();
                let hint = did_you_mean(name, names);
                return Err(DslError::new(
                    DslErrorKind::UnknownWatchName {
                        name: name.clone(),
                        hint,
                    },
                    at.clone(),
                ));
            }
            Ok(())
        }
        WatchWhen::Inline(p) => {
            // Insert the inline predicate under a synthetic name and re-run the
            // graph validator so it can follow references back into the named
            // map. The synthetic key is guaranteed not to clash.
            let synthetic = "__vcli_inline__".to_string();
            if predicates.contains_key(&synthetic) {
                return Err(DslError::new(
                    DslErrorKind::CanonicalizationFailed(
                        "reserved name __vcli_inline__ collides with a user predicate".into(),
                    ),
                    at.clone(),
                ));
            }
            let mut merged = predicates.clone();
            merged.insert(synthetic, (**p).clone());
            validate_predicate_graph(&merged).map_err(|mut e| {
                // Rewrite the error path from /predicates/__vcli_inline__/... to
                // the watch's /watches/N/when/...
                let synthetic_prefix = "/predicates/__vcli_inline__";
                let orig_path = e.path.to_string();
                if let Some(remainder) = orig_path.strip_prefix(synthetic_prefix) {
                    let mut new_path = at.clone();
                    for seg in remainder.split('/').filter(|s| !s.is_empty()) {
                        new_path = new_path.key(seg);
                    }
                    e.path = new_path;
                }
                e
            })
        }
    }
}

fn validate_lifetime(
    l: &Lifetime,
    predicates: &BTreeMap<String, PredicateKind>,
    at: &JsonPath,
) -> Result<(), DslError> {
    if let Lifetime::UntilPredicate { name } = l {
        if !predicates.contains_key(name) {
            let names: Vec<&str> = predicates.keys().map(String::as_str).collect();
            let hint = did_you_mean(name, names);
            return Err(DslError::new(
                DslErrorKind::UnknownPredicateName {
                    name: name.clone(),
                    hint,
                },
                at.key("name"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vcli_core::Watch;

    fn preds() -> BTreeMap<String, PredicateKind> {
        serde_json::from_value(json!({
            "skip_visible": {"kind":"template","image":"x.png","confidence":0.9,
                             "region":{"kind":"absolute","box":{"x":0,"y":0,"w":10,"h":10}}}
        }))
        .unwrap()
    }

    fn watches_of(v: serde_json::Value) -> Vec<Watch> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn valid_by_name_watch_ok() {
        let p = preds();
        let w = watches_of(json!([
            {"when":"skip_visible","do":[{"kind":"click","at":"$skip_visible.match.center"}],
             "lifetime":{"kind":"persistent"}}
        ]));
        validate_watches(&w, &p).unwrap();
    }

    #[test]
    fn unknown_when_name_reports_path_and_hint() {
        let p = preds();
        let w = watches_of(json!([
            {"when":"skp_visible","do":[],"lifetime":{"kind":"persistent"}}
        ]));
        let e = validate_watches(&w, &p).unwrap_err();
        assert_eq!(e.path.to_string(), "/watches/0/when");
        match e.kind {
            DslErrorKind::UnknownWatchName { name, hint } => {
                assert_eq!(name, "skp_visible");
                assert_eq!(hint.as_deref(), Some("skip_visible"));
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn inline_predicate_with_bad_ref_reports_rewritten_path() {
        let p = preds();
        let w = watches_of(json!([
            {
                "when": {"kind":"not","of":"nope"},
                "do": [],
                "lifetime":{"kind":"persistent"}
            }
        ]));
        let e = validate_watches(&w, &p).unwrap_err();
        assert_eq!(e.path.to_string(), "/watches/0/when/of");
    }

    #[test]
    fn inline_template_with_relative_to_named_predicate_ok() {
        let p = preds();
        let w = watches_of(json!([
            {
                "when": {"kind":"template","image":"y.png","confidence":0.9,
                         "region":{"kind":"relative_to","predicate":"skip_visible"}},
                "do": [],
                "lifetime":{"kind":"persistent"}
            }
        ]));
        validate_watches(&w, &p).unwrap();
    }

    #[test]
    fn body_only_step_in_watch_reports_under_do() {
        let p = preds();
        let w = watches_of(json!([
            {"when":"skip_visible",
             "do":[{"kind":"sleep_ms","ms":100}],
             "lifetime":{"kind":"persistent"}}
        ]));
        let e = validate_watches(&w, &p).unwrap_err();
        assert_eq!(e.path.to_string(), "/watches/0/do/0");
        assert!(matches!(e.kind, DslErrorKind::BodyOnlyStepInWatch { .. }));
    }

    #[test]
    fn lifetime_until_predicate_unknown_rejected() {
        let p = preds();
        let w = watches_of(json!([
            {"when":"skip_visible","do":[],
             "lifetime":{"kind":"until_predicate","name":"missing"}}
        ]));
        let e = validate_watches(&w, &p).unwrap_err();
        assert_eq!(e.path.to_string(), "/watches/0/lifetime/name");
        assert!(matches!(e.kind, DslErrorKind::UnknownPredicateName { .. }));
    }

    #[test]
    fn lifetime_timeout_ms_ignores_predicates() {
        let p = preds();
        let w = watches_of(json!([
            {"when":"skip_visible","do":[],
             "lifetime":{"kind":"timeout_ms","ms":5000}}
        ]));
        validate_watches(&w, &p).unwrap();
    }
}
