//! Validation of the `predicates` map: collect names, reject empty logical
//! operands, verify every name referenced inside a logical / elapsed / region
//! predicate exists. Cycle detection lands in Task 7.

use std::collections::BTreeMap;

use vcli_core::{Anchor, PredicateKind, Region};

use crate::error::{DslError, DslErrorKind};
use crate::hint::did_you_mean;
use crate::path::JsonPath;

/// Walk every named predicate and verify references + shape.
///
/// Returns `Ok(())` on success. Collects the first error only (callers wanting
/// all errors can call this repeatedly after fixes; v0 prioritizes clarity over
/// error-list batching).
///
/// # Errors
///
/// Returns [`DslError`] on the first violation found.
pub fn validate_predicate_shape(
    predicates: &BTreeMap<String, PredicateKind>,
) -> Result<(), DslError> {
    let names: Vec<&str> = predicates.keys().map(String::as_str).collect();
    for (name, pred) in predicates {
        let base = JsonPath::root().key("predicates").key(name);
        check_predicate_references(pred, &names, &base)?;
    }
    Ok(())
}

fn check_predicate_references(
    pred: &PredicateKind,
    names: &[&str],
    at: &JsonPath,
) -> Result<(), DslError> {
    match pred {
        PredicateKind::AllOf { of } => {
            if of.is_empty() {
                return Err(DslError::new(
                    DslErrorKind::EmptyLogicalOp { op: "all_of" },
                    at.key("of"),
                ));
            }
            for (i, n) in of.iter().enumerate() {
                resolve_name(n, names, &at.key("of").index(i))?;
            }
        }
        PredicateKind::AnyOf { of } => {
            if of.is_empty() {
                return Err(DslError::new(
                    DslErrorKind::EmptyLogicalOp { op: "any_of" },
                    at.key("of"),
                ));
            }
            for (i, n) in of.iter().enumerate() {
                resolve_name(n, names, &at.key("of").index(i))?;
            }
        }
        PredicateKind::Not { of } => {
            resolve_name(of, names, &at.key("of"))?;
        }
        PredicateKind::ElapsedMsSinceTrue { predicate, .. } => {
            resolve_elapsed(predicate, names, &at.key("predicate"))?;
        }
        PredicateKind::Template { region, .. } | PredicateKind::PixelDiff { region, .. } => {
            check_region_references(region, names, &at.key("region"))?;
        }
        PredicateKind::ColorAt { .. } => {}
    }
    Ok(())
}

fn check_region_references(
    region: &Region,
    names: &[&str],
    at: &JsonPath,
) -> Result<(), DslError> {
    if let Region::RelativeTo { predicate, anchor, .. } = region {
        let _ = Anchor::Match; // compile-time proof we know Anchor's shape
        let _ = anchor;
        resolve_name(predicate, names, &at.key("predicate"))?;
    }
    Ok(())
}

fn resolve_name(name: &str, names: &[&str], at: &JsonPath) -> Result<(), DslError> {
    if names.iter().any(|n| *n == name) {
        return Ok(());
    }
    let hint = did_you_mean(name, names.iter().copied());
    Err(DslError::new(
        DslErrorKind::UnknownPredicateName {
            name: name.to_string(),
            hint,
        },
        at.clone(),
    ))
}

fn resolve_elapsed(name: &str, names: &[&str], at: &JsonPath) -> Result<(), DslError> {
    if names.iter().any(|n| *n == name) {
        return Ok(());
    }
    Err(DslError::new(
        DslErrorKind::ElapsedReferenceMissing {
            name: name.to_string(),
        },
        at.clone(),
    ))
}

/// Full validation of the predicate map: shape + cycle detection over the
/// merged dependency graph (Decision 1.3). Edges come from `all_of`/`any_of`/
/// `not`, `elapsed_ms_since_true.predicate`, and `region: relative_to.predicate`.
///
/// # Errors
///
/// Returns [`DslError::kind = PredicateCycle`] on any cycle, or any shape /
/// reference error from [`validate_predicate_shape`].
pub fn validate_predicate_graph(
    predicates: &BTreeMap<String, PredicateKind>,
) -> Result<(), DslError> {
    validate_predicate_shape(predicates)?;

    // Iterative DFS: WHITE unvisited, GRAY in-stack, BLACK done.
    #[derive(Copy, Clone, PartialEq, Eq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut color: BTreeMap<&str, Color> = predicates
        .keys()
        .map(|k| (k.as_str(), Color::White))
        .collect();

    for start in predicates.keys() {
        if color[start.as_str()] != Color::White {
            continue;
        }
        // Stack entries: (name, iterator of outgoing edges as Vec pre-collected).
        let mut stack: Vec<(&str, std::vec::IntoIter<String>)> = Vec::new();
        let edges = edges_of(&predicates[start]);
        color.insert(start.as_str(), Color::Gray);
        stack.push((start.as_str(), edges.into_iter()));
        // Path vec mirrors stack for cycle reporting.
        let mut path: Vec<&str> = vec![start.as_str()];

        while let Some((_cur, iter)) = stack.last_mut() {
            if let Some(next) = iter.next() {
                // Skip edges to names that don't exist — shape pass already
                // erred on those, but validate_predicate_shape collects only
                // the first error so we remain defensive here.
                let Some(next_key) = predicates.keys().find(|k| k.as_str() == next.as_str()) else {
                    continue;
                };
                let next_str: &str = next_key.as_str();
                match color[next_str] {
                    Color::White => {
                        color.insert(next_str, Color::Gray);
                        let e = edges_of(&predicates[next_str]);
                        path.push(next_str);
                        stack.push((next_str, e.into_iter()));
                    }
                    Color::Gray => {
                        // Cycle found. Trim path to the position of next_str.
                        let start_idx = path.iter().position(|s| *s == next_str).unwrap_or(0);
                        let mut cycle: Vec<String> =
                            path[start_idx..].iter().map(|s| (*s).to_string()).collect();
                        cycle.push(next_str.to_string());
                        return Err(DslError::new(
                            DslErrorKind::PredicateCycle { cycle },
                            JsonPath::root().key("predicates"),
                        ));
                    }
                    Color::Black => {}
                }
            } else {
                // Exhausted this node's edges; mark black, pop.
                let (cur_name, _) = stack.pop().unwrap();
                color.insert(cur_name, Color::Black);
                path.pop();
            }
        }
    }
    Ok(())
}

fn edges_of(p: &PredicateKind) -> Vec<String> {
    let mut out = Vec::new();
    match p {
        PredicateKind::AllOf { of } | PredicateKind::AnyOf { of } => {
            out.extend(of.iter().cloned());
        }
        PredicateKind::Not { of } => out.push(of.clone()),
        PredicateKind::ElapsedMsSinceTrue { predicate, .. } => out.push(predicate.clone()),
        PredicateKind::Template { region, .. } | PredicateKind::PixelDiff { region, .. } => {
            if let Region::RelativeTo { predicate, .. } = region {
                out.push(predicate.clone());
            }
        }
        PredicateKind::ColorAt { .. } => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vcli_core::PredicateKind;

    fn parse_predicates(v: serde_json::Value) -> BTreeMap<String, PredicateKind> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn empty_map_is_valid() {
        let p: BTreeMap<String, PredicateKind> = BTreeMap::new();
        assert!(validate_predicate_shape(&p).is_ok());
    }

    #[test]
    fn all_of_with_known_names_ok() {
        let p = parse_predicates(json!({
            "a": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":1},
            "b": {"kind":"color_at","point":{"x":1,"y":1},"rgb":[0,0,0],"tolerance":1},
            "c": {"kind":"all_of","of":["a","b"]}
        }));
        assert!(validate_predicate_shape(&p).is_ok());
    }

    #[test]
    fn all_of_with_unknown_reports_path() {
        let p = parse_predicates(json!({
            "a": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":1},
            "c": {"kind":"all_of","of":["a","zzz"]}
        }));
        let e = validate_predicate_shape(&p).unwrap_err();
        match e.kind {
            DslErrorKind::UnknownPredicateName { name, .. } => assert_eq!(name, "zzz"),
            other => panic!("wrong kind: {other:?}"),
        }
        assert_eq!(e.path.to_string(), "/predicates/c/of/1");
    }

    #[test]
    fn any_of_empty_rejected() {
        let p = parse_predicates(json!({
            "c": {"kind":"any_of","of":[]}
        }));
        let e = validate_predicate_shape(&p).unwrap_err();
        match e.kind {
            DslErrorKind::EmptyLogicalOp { op } => assert_eq!(op, "any_of"),
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn not_with_unknown_reports_at_of() {
        let p = parse_predicates(json!({
            "c": {"kind":"not","of":"nope"}
        }));
        let e = validate_predicate_shape(&p).unwrap_err();
        assert_eq!(e.path.to_string(), "/predicates/c/of");
        assert!(matches!(e.kind, DslErrorKind::UnknownPredicateName { .. }));
    }

    #[test]
    fn elapsed_with_unknown_reports_distinct_error() {
        let p = parse_predicates(json!({
            "c": {"kind":"elapsed_ms_since_true","predicate":"nope","ms":100}
        }));
        let e = validate_predicate_shape(&p).unwrap_err();
        assert!(matches!(e.kind, DslErrorKind::ElapsedReferenceMissing { .. }));
        assert_eq!(e.path.to_string(), "/predicates/c/predicate");
    }

    #[test]
    fn template_with_relative_region_resolved() {
        let p = parse_predicates(json!({
            "anchor": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":1},
            "t": {"kind":"template","image":"x.png","confidence":0.9,
                  "region":{"kind":"relative_to","predicate":"anchor"}}
        }));
        assert!(validate_predicate_shape(&p).is_ok());
    }

    #[test]
    fn template_with_unknown_relative_ref_reports() {
        let p = parse_predicates(json!({
            "t": {"kind":"template","image":"x.png","confidence":0.9,
                  "region":{"kind":"relative_to","predicate":"absent"}}
        }));
        let e = validate_predicate_shape(&p).unwrap_err();
        assert_eq!(e.path.to_string(), "/predicates/t/region/predicate");
    }

    #[test]
    fn did_you_mean_hint_fires_on_single_edit() {
        let p = parse_predicates(json!({
            "skip_visible": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":1},
            "c": {"kind":"not","of":"skp_visible"}
        }));
        let e = validate_predicate_shape(&p).unwrap_err();
        match e.kind {
            DslErrorKind::UnknownPredicateName { hint, .. } => {
                assert_eq!(hint.as_deref(), Some("skip_visible"));
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    // --- Task 7 cycle detection tests ---

    #[test]
    fn self_cycle_via_not_is_rejected() {
        let p = parse_predicates(json!({
            "x": {"kind":"not","of":"x"}
        }));
        let e = validate_predicate_graph(&p).unwrap_err();
        match e.kind {
            DslErrorKind::PredicateCycle { cycle } => {
                assert_eq!(cycle.first().map(String::as_str), Some("x"));
                assert_eq!(cycle.last().map(String::as_str), Some("x"));
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn two_node_cycle_all_of_not() {
        let p = parse_predicates(json!({
            "a": {"kind":"not","of":"b"},
            "b": {"kind":"all_of","of":["a"]}
        }));
        let e = validate_predicate_graph(&p).unwrap_err();
        assert!(matches!(e.kind, DslErrorKind::PredicateCycle { .. }));
    }

    #[test]
    fn cycle_through_relative_to_region_detected() {
        let p = parse_predicates(json!({
            "a": {"kind":"template","image":"x.png","confidence":0.9,
                  "region":{"kind":"relative_to","predicate":"b"}},
            "b": {"kind":"template","image":"y.png","confidence":0.9,
                  "region":{"kind":"relative_to","predicate":"a"}}
        }));
        let e = validate_predicate_graph(&p).unwrap_err();
        assert!(matches!(e.kind, DslErrorKind::PredicateCycle { .. }));
    }

    #[test]
    fn cycle_through_elapsed_ms_since_true_detected() {
        let p = parse_predicates(json!({
            "a": {"kind":"elapsed_ms_since_true","predicate":"b","ms":100},
            "b": {"kind":"not","of":"a"}
        }));
        let e = validate_predicate_graph(&p).unwrap_err();
        assert!(matches!(e.kind, DslErrorKind::PredicateCycle { .. }));
    }

    #[test]
    fn acyclic_dag_passes_graph_check() {
        let p = parse_predicates(json!({
            "leaf": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":1},
            "mid":  {"kind":"not","of":"leaf"},
            "root": {"kind":"all_of","of":["leaf","mid"]}
        }));
        assert!(validate_predicate_graph(&p).is_ok());
    }
}
