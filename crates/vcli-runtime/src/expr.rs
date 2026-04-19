//! Expression resolver: `$<name>.match.center` and `$<name>.match.bbox`.

use vcli_core::geom::{Point, Rect};
use vcli_core::predicate::PredicateResult;

use crate::error::RuntimeError;

/// Parts of an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedExpr<'a> {
    /// Referenced predicate name.
    pub predicate: &'a str,
    /// Accessor path (`.match.center` / `.match.bbox`).
    pub accessor: Accessor,
}

/// Supported accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accessor {
    /// `.match.center`
    MatchCenter,
    /// `.match.bbox`
    MatchBbox,
}

/// Parse an expression string. Returns [`RuntimeError::ExpressionUnresolved`]
/// on malformed input.
///
/// # Errors
///
/// Returns `ExpressionUnresolved` for non-`$...` strings or unknown accessors.
pub fn parse(s: &str) -> Result<ParsedExpr<'_>, RuntimeError> {
    let rest = s
        .strip_prefix('$')
        .ok_or_else(|| RuntimeError::ExpressionUnresolved(format!("expected '$' prefix: {s}")))?;
    let (pred, tail) = rest
        .split_once('.')
        .ok_or_else(|| RuntimeError::ExpressionUnresolved(format!("expected '.<accessor>': {s}")))?;
    if pred.is_empty() {
        return Err(RuntimeError::ExpressionUnresolved(format!(
            "empty predicate name: {s}"
        )));
    }
    let accessor = match tail {
        "match.center" => Accessor::MatchCenter,
        "match.bbox" => Accessor::MatchBbox,
        other => {
            return Err(RuntimeError::ExpressionUnresolved(format!(
                "unknown accessor '{other}'"
            )));
        }
    };
    Ok(ParsedExpr { predicate: pred, accessor })
}

/// Resolve a parsed expression's center against a predicate result.
///
/// Uses `(w-1)/2` / `(h-1)/2` offsets so a 40-wide rect centers on pixel 19,
/// not 20 — spec §222's inclusive-pixel definition.
///
/// # Errors
///
/// Returns `ExpressionUnresolved` if the result has no `match_data` (the
/// predicate was truthy but non-spatial, e.g. `color_at`).
pub fn resolve_center(r: &PredicateResult) -> Result<Point, RuntimeError> {
    let md = r
        .match_data
        .as_ref()
        .ok_or_else(|| RuntimeError::ExpressionUnresolved("predicate has no match_data".into()))?;
    let w_minus_1 = u32::try_from(md.bbox.w.max(1) - 1).unwrap_or(0);
    let h_minus_1 = u32::try_from(md.bbox.h.max(1) - 1).unwrap_or(0);
    Ok(Point {
        x: md.bbox.x + i32::try_from(w_minus_1 / 2).unwrap_or(0),
        y: md.bbox.y + i32::try_from(h_minus_1 / 2).unwrap_or(0),
    })
}

/// Resolve a parsed expression's bbox against a predicate result.
///
/// # Errors
///
/// Returns `ExpressionUnresolved` when there is no `match_data`.
pub fn resolve_bbox(r: &PredicateResult) -> Result<Rect, RuntimeError> {
    let md = r
        .match_data
        .as_ref()
        .ok_or_else(|| RuntimeError::ExpressionUnresolved("predicate has no match_data".into()))?;
    Ok(md.bbox)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::predicate::{Confidence, MatchData};

    #[test]
    fn parse_match_center() {
        let p = parse("$skip.match.center").unwrap();
        assert_eq!(p.predicate, "skip");
        assert_eq!(p.accessor, Accessor::MatchCenter);
    }

    #[test]
    fn parse_match_bbox() {
        let p = parse("$x.match.bbox").unwrap();
        assert_eq!(p.accessor, Accessor::MatchBbox);
    }

    #[test]
    fn parse_rejects_missing_dollar() {
        assert!(matches!(
            parse("skip.match.center"),
            Err(RuntimeError::ExpressionUnresolved(_))
        ));
    }

    #[test]
    fn parse_rejects_unknown_accessor() {
        assert!(matches!(
            parse("$skip.match.topleft"),
            Err(RuntimeError::ExpressionUnresolved(_))
        ));
    }

    #[test]
    fn resolve_center_averages_bbox() {
        let r = PredicateResult {
            truthy: true,
            at: 0,
            match_data: Some(MatchData {
                bbox: Rect { x: 10, y: 20, w: 40, h: 20 },
                confidence: Confidence(0.9),
            }),
        };
        assert_eq!(resolve_center(&r).unwrap(), Point { x: 29, y: 29 });
    }

    #[test]
    fn resolve_without_match_data_errors() {
        let r = PredicateResult { truthy: true, at: 0, match_data: None };
        assert!(resolve_center(&r).is_err());
    }
}
