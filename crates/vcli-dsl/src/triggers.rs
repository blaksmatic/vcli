//! Validate the program trigger. `on_submit` and `manual` are always valid;
//! `on_predicate` requires name resolution.

use std::collections::BTreeMap;

use vcli_core::{PredicateKind, Trigger};

use crate::error::{DslError, DslErrorKind};
use crate::hint::did_you_mean;
use crate::path::JsonPath;

/// Validate the trigger against the predicate map.
///
/// # Errors
///
/// Returns [`DslError`] with kind [`DslErrorKind::UnknownTriggerName`] when
/// `on_predicate.name` doesn't resolve.
pub fn validate_trigger(
    trigger: &Trigger,
    predicates: &BTreeMap<String, PredicateKind>,
) -> Result<(), DslError> {
    if let Trigger::OnPredicate { name } = trigger {
        if !predicates.contains_key(name) {
            let names: Vec<&str> = predicates.keys().map(String::as_str).collect();
            let hint = did_you_mean(name, names);
            return Err(DslError::new(
                DslErrorKind::UnknownTriggerName {
                    name: name.clone(),
                    hint,
                },
                JsonPath::root().key("trigger").key("name"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn preds() -> BTreeMap<String, PredicateKind> {
        serde_json::from_value(json!({
            "ready": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":1}
        }))
        .unwrap()
    }

    #[test]
    fn on_submit_always_ok() {
        validate_trigger(&Trigger::OnSubmit, &BTreeMap::new()).unwrap();
    }

    #[test]
    fn manual_always_ok() {
        validate_trigger(&Trigger::Manual, &BTreeMap::new()).unwrap();
    }

    #[test]
    fn on_predicate_with_known_name_ok() {
        validate_trigger(
            &Trigger::OnPredicate {
                name: "ready".into(),
            },
            &preds(),
        )
        .unwrap();
    }

    #[test]
    fn on_predicate_with_unknown_reports_path() {
        let e = validate_trigger(
            &Trigger::OnPredicate {
                name: "nope".into(),
            },
            &preds(),
        )
        .unwrap_err();
        assert_eq!(e.path.to_string(), "/trigger/name");
        match e.kind {
            DslErrorKind::UnknownTriggerName { name, .. } => assert_eq!(name, "nope"),
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn on_predicate_hint_fires_on_typo() {
        let e = validate_trigger(
            &Trigger::OnPredicate {
                name: "rady".into(),
            },
            &preds(),
        )
        .unwrap_err();
        match e.kind {
            DslErrorKind::UnknownTriggerName { hint, .. } => {
                assert_eq!(hint.as_deref(), Some("ready"));
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }
}
