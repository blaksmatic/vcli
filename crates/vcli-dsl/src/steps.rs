//! Validate a sequence of steps. The validator distinguishes body context (all
//! step kinds legal) from watch context (no `wait_for`/`assert`/`sleep_ms`).
//! Inside either context it re-checks target expressions, absolute coordinates,
//! and predicate-name references in `wait_for`/`assert`.

use std::collections::BTreeMap;

use vcli_core::{PredicateKind, Step, Target};

use crate::error::{DslError, DslErrorKind};
use crate::expression::Expression;
use crate::hint::did_you_mean;
use crate::path::JsonPath;

/// Validate a sequence of body steps.
///
/// # Errors
///
/// Returns [`DslError`] on any offending step.
pub fn validate_body_steps(
    steps: &[Step],
    predicates: &BTreeMap<String, PredicateKind>,
    base: &JsonPath,
) -> Result<(), DslError> {
    for (i, s) in steps.iter().enumerate() {
        let at = base.index(i);
        validate_step(s, predicates, /* in_watch = */ false, &at)?;
    }
    Ok(())
}

/// Validate a sequence of watch `do` steps. Disallows body-only step kinds.
///
/// # Errors
///
/// Returns [`DslError`] on any offending step, including body-only kinds.
pub fn validate_watch_steps(
    steps: &[Step],
    predicates: &BTreeMap<String, PredicateKind>,
    base: &JsonPath,
) -> Result<(), DslError> {
    for (i, s) in steps.iter().enumerate() {
        let at = base.index(i);
        validate_step(s, predicates, /* in_watch = */ true, &at)?;
    }
    Ok(())
}

fn validate_step(
    s: &Step,
    predicates: &BTreeMap<String, PredicateKind>,
    in_watch: bool,
    at: &JsonPath,
) -> Result<(), DslError> {
    match s {
        Step::Move { at: tgt } | Step::Click { at: tgt, .. } | Step::Scroll { at: tgt, .. } => {
            validate_target(tgt, predicates, &at.key("at"))?;
        }
        Step::Type { .. } | Step::Key { .. } => {}
        Step::WaitFor { predicate, .. } => {
            if in_watch {
                return Err(DslError::new(
                    DslErrorKind::BodyOnlyStepInWatch { kind: "wait_for" },
                    at.clone(),
                ));
            }
            resolve_step_name(predicate, predicates, &at.key("predicate"))?;
        }
        Step::Assert { predicate, .. } => {
            if in_watch {
                return Err(DslError::new(
                    DslErrorKind::BodyOnlyStepInWatch { kind: "assert" },
                    at.clone(),
                ));
            }
            resolve_step_name(predicate, predicates, &at.key("predicate"))?;
        }
        Step::SleepMs { .. } => {
            if in_watch {
                return Err(DslError::new(
                    DslErrorKind::BodyOnlyStepInWatch { kind: "sleep_ms" },
                    at.clone(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_target(
    t: &Target,
    predicates: &BTreeMap<String, PredicateKind>,
    at: &JsonPath,
) -> Result<(), DslError> {
    match t {
        Target::Absolute(p) => {
            if p.x < 0 || p.y < 0 {
                return Err(DslError::new(
                    DslErrorKind::NegativeAbsoluteTarget { x: p.x, y: p.y },
                    at.clone(),
                ));
            }
            Ok(())
        }
        Target::Expression(raw) => {
            let expr = Expression::parse(raw, at)?;
            match predicates.get(&expr.name) {
                None => {
                    let names: Vec<&str> = predicates.keys().map(String::as_str).collect();
                    let hint = did_you_mean(&expr.name, names.into_iter());
                    Err(DslError::new(
                        DslErrorKind::UnknownPredicateName {
                            name: expr.name,
                            hint,
                        },
                        at.clone(),
                    ))
                }
                Some(PredicateKind::AllOf { .. })
                | Some(PredicateKind::AnyOf { .. })
                | Some(PredicateKind::Not { .. })
                | Some(PredicateKind::ElapsedMsSinceTrue { .. }) => Err(DslError::new(
                    DslErrorKind::ExpressionOnLogicalPredicate { name: expr.name },
                    at.clone(),
                )),
                Some(_) => Ok(()),
            }
        }
    }
}

fn resolve_step_name(
    name: &str,
    predicates: &BTreeMap<String, PredicateKind>,
    at: &JsonPath,
) -> Result<(), DslError> {
    if predicates.contains_key(name) {
        return Ok(());
    }
    let names: Vec<&str> = predicates.keys().map(String::as_str).collect();
    let hint = did_you_mean(name, names.into_iter());
    Err(DslError::new(
        DslErrorKind::UnknownPredicateName {
            name: name.to_string(),
            hint,
        },
        at.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn preds() -> BTreeMap<String, PredicateKind> {
        serde_json::from_value(json!({
            "skip_visible": {"kind":"template","image":"x.png","confidence":0.9,
                             "region":{"kind":"absolute","box":{"x":0,"y":0,"w":10,"h":10}}},
            "logic":        {"kind":"not","of":"skip_visible"}
        }))
        .unwrap()
    }

    fn steps_of(v: serde_json::Value) -> Vec<Step> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn valid_click_expression_target_ok() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"click","at":"$skip_visible.match.center"}
        ]));
        validate_body_steps(&s, &p, &JsonPath::root().key("body")).unwrap();
    }

    #[test]
    fn click_with_unknown_predicate_reports_path() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"click","at":"$nope.match.center"}
        ]));
        let e = validate_body_steps(&s, &p, &JsonPath::root().key("body")).unwrap_err();
        assert_eq!(e.path.to_string(), "/body/0/at");
        assert!(matches!(e.kind, DslErrorKind::UnknownPredicateName { .. }));
    }

    #[test]
    fn negative_absolute_target_rejected() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"click","at":{"x":-1,"y":5}}
        ]));
        let e = validate_body_steps(&s, &p, &JsonPath::root().key("body")).unwrap_err();
        match e.kind {
            DslErrorKind::NegativeAbsoluteTarget { x, y } => {
                assert_eq!(x, -1);
                assert_eq!(y, 5);
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn expression_on_logical_predicate_rejected() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"click","at":"$logic.match.center"}
        ]));
        let e = validate_body_steps(&s, &p, &JsonPath::root().key("body")).unwrap_err();
        assert!(matches!(e.kind, DslErrorKind::ExpressionOnLogicalPredicate { .. }));
    }

    #[test]
    fn wait_for_in_body_ok() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"wait_for","predicate":"skip_visible","timeout_ms":1000}
        ]));
        validate_body_steps(&s, &p, &JsonPath::root().key("body")).unwrap();
    }

    #[test]
    fn wait_for_in_watch_rejected() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"wait_for","predicate":"skip_visible","timeout_ms":1000}
        ]));
        let e = validate_watch_steps(&s, &p, &JsonPath::root().key("watches").index(0).key("do"))
            .unwrap_err();
        match e.kind {
            DslErrorKind::BodyOnlyStepInWatch { kind } => assert_eq!(kind, "wait_for"),
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn assert_in_watch_rejected() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"assert","predicate":"skip_visible"}
        ]));
        let e = validate_watch_steps(&s, &p, &JsonPath::root().key("watches").index(0).key("do"))
            .unwrap_err();
        assert!(matches!(
            e.kind,
            DslErrorKind::BodyOnlyStepInWatch { kind: "assert" }
        ));
    }

    #[test]
    fn sleep_ms_in_watch_rejected() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"sleep_ms","ms":100}
        ]));
        let e = validate_watch_steps(&s, &p, &JsonPath::root().key("watches").index(0).key("do"))
            .unwrap_err();
        assert!(matches!(
            e.kind,
            DslErrorKind::BodyOnlyStepInWatch { kind: "sleep_ms" }
        ));
    }

    #[test]
    fn assert_with_unknown_predicate_rejected() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"assert","predicate":"nope"}
        ]));
        let e = validate_body_steps(&s, &p, &JsonPath::root().key("body")).unwrap_err();
        assert_eq!(e.path.to_string(), "/body/0/predicate");
    }

    #[test]
    fn type_and_key_never_reference_predicates() {
        let p = preds();
        let s = steps_of(json!([
            {"kind":"type","text":"hello"},
            {"kind":"key","key":"s","modifiers":["cmd"]}
        ]));
        validate_body_steps(&s, &p, &JsonPath::root().key("body")).unwrap();
    }
}
