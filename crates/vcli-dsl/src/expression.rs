//! Minimal dotted-path expression language for step targets.
//!
//! Grammar (v0):
//!   Expression := '$' Name '.' 'match' '.' Accessor
//!   Name       := [A-Za-z_][A-Za-z0-9_]*
//!   Accessor   := 'center' | '`top_left`' | 'box' | 'confidence'
//!
//! Parser errors carry a static reason string; callers attach a `JsonPath`
//! for context.

use crate::error::{DslError, DslErrorKind};
use crate::path::JsonPath;

/// Accessors supported in v0 expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accessor {
    /// `$p.match.center` → Point.
    Center,
    /// `$p.match.top_left` → Point.
    TopLeft,
    /// `$p.match.box` → Rect.
    Box,
    /// `$p.match.confidence` → f64.
    Confidence,
}

/// Parsed expression. The predicate `name` is always non-empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expression {
    /// Referenced predicate name.
    pub name: String,
    /// Accessor invoked after `.match.`.
    pub accessor: Accessor,
}

impl Expression {
    /// Parse an expression string. Input must start with `$` and match the grammar.
    ///
    /// # Errors
    ///
    /// Returns [`DslError`] with kind [`DslErrorKind::MalformedExpression`].
    pub fn parse(src: &str, at: &JsonPath) -> Result<Self, DslError> {
        let rest = src
            .strip_prefix('$')
            .ok_or_else(|| err("expression must begin with '$'", src, at))?;
        let mut parts = rest.split('.');
        let name = parts
            .next()
            .filter(|n| !n.is_empty())
            .ok_or_else(|| err("missing predicate name after '$'", src, at))?;
        if !is_valid_name(name) {
            return Err(err(
                "predicate name must match [A-Za-z_][A-Za-z0-9_]*",
                src,
                at,
            ));
        }
        let match_kw = parts
            .next()
            .ok_or_else(|| err("missing '.match'", src, at))?;
        if match_kw != "match" {
            return Err(err("expected '.match' after predicate name", src, at));
        }
        let acc = parts
            .next()
            .ok_or_else(|| err("missing accessor", src, at))?;
        if parts.next().is_some() {
            return Err(err("trailing segments after accessor", src, at));
        }
        let accessor = match acc {
            "center" => Accessor::Center,
            "top_left" => Accessor::TopLeft,
            "box" => Accessor::Box,
            "confidence" => Accessor::Confidence,
            _ => return Err(err("unknown accessor", src, at)),
        };
        Ok(Self {
            name: name.to_string(),
            accessor,
        })
    }
}

fn err(reason: &'static str, raw: &str, at: &JsonPath) -> DslError {
    DslError::new(
        DslErrorKind::MalformedExpression {
            reason,
            raw: raw.to_string(),
        },
        at.clone(),
    )
}

fn is_valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Expression, DslError> {
        Expression::parse(s, &JsonPath::root().key("body").index(0).key("at"))
    }

    #[test]
    fn parses_center() {
        let e = p("$skip_visible.match.center").unwrap();
        assert_eq!(e.name, "skip_visible");
        assert_eq!(e.accessor, Accessor::Center);
    }

    #[test]
    fn parses_all_accessors() {
        assert_eq!(p("$x.match.center").unwrap().accessor, Accessor::Center);
        assert_eq!(p("$x.match.top_left").unwrap().accessor, Accessor::TopLeft);
        assert_eq!(p("$x.match.box").unwrap().accessor, Accessor::Box);
        assert_eq!(
            p("$x.match.confidence").unwrap().accessor,
            Accessor::Confidence
        );
    }

    #[test]
    fn rejects_missing_dollar() {
        let e = p("x.match.center").unwrap_err();
        match e.kind {
            DslErrorKind::MalformedExpression { reason, .. } => {
                assert!(reason.contains("'$'"));
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_accessor() {
        let e = p("$x.match.area").unwrap_err();
        match e.kind {
            DslErrorKind::MalformedExpression { reason, .. } => {
                assert!(reason.contains("accessor"));
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_match_keyword() {
        assert!(p("$x.mtach.center").is_err());
    }

    #[test]
    fn rejects_trailing_segments() {
        assert!(p("$x.match.center.extra").is_err());
    }

    #[test]
    fn rejects_empty_name() {
        assert!(p("$.match.center").is_err());
    }

    #[test]
    fn rejects_non_identifier_name() {
        assert!(p("$1bad.match.center").is_err());
        assert!(p("$bad-name.match.center").is_err());
    }

    #[test]
    fn path_in_error_is_preserved() {
        let e = p("nope").unwrap_err();
        assert_eq!(e.path.to_string(), "/body/0/at");
    }
}
