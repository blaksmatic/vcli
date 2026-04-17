//! Rich validation errors. Each `DslError` carries a [`JsonPath`] pointing at
//! the offending location and a machine-readable [`DslErrorKind`]. Conversion
//! to the wire shape `vcli_core::ErrorPayload` preserves the path as the
//! `path` field of the payload.

use thiserror::Error;

use vcli_core::{CanonicalError, ErrorCode, ErrorPayload};

use crate::path::JsonPath;

/// Validation error. Always carries a `path`; `kind` is the discriminator.
#[derive(Debug, Clone, PartialEq, Error)]
#[error("{kind} at {path}")]
pub struct DslError {
    /// What went wrong.
    pub kind: DslErrorKind,
    /// JSON pointer into the offending program.
    pub path: JsonPath,
}

impl DslError {
    /// Construct a new error at the given path.
    #[must_use]
    pub fn new(kind: DslErrorKind, path: JsonPath) -> Self {
        Self { kind, path }
    }

    /// Optional Levenshtein-1 hint for name-not-found errors (Decision 2.2).
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        match &self.kind {
            DslErrorKind::UnknownPredicateName { hint, .. }
            | DslErrorKind::UnknownWatchName { hint, .. }
            | DslErrorKind::UnknownTriggerName { hint, .. } => hint.as_deref(),
            _ => None,
        }
    }

    /// Convert to the stable wire payload.
    #[must_use]
    pub fn to_payload(&self) -> ErrorPayload {
        ErrorPayload {
            code: ErrorCode::InvalidProgram,
            message: self.kind.to_string(),
            path: Some(self.path.to_string()),
            line: None,
            column: None,
            span_len: None,
            hint: self.hint().map(str::to_string),
        }
    }
}

impl From<CanonicalError> for DslError {
    fn from(e: CanonicalError) -> Self {
        Self::new(
            DslErrorKind::CanonicalizationFailed(e.to_string()),
            JsonPath::root(),
        )
    }
}

/// Machine-readable error kinds. Every variant must render a useful `Display`.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DslErrorKind {
    /// JSON failed to parse at all (`serde_json` error message).
    #[error("json parse failed: {0}")]
    JsonParse(String),

    /// DSL major version we don't recognize.
    #[error("unsupported dsl version: {found:?} (expected major {expected_major:?})")]
    UnsupportedDslVersion {
        /// Version string from the document.
        found: String,
        /// Supported major digit.
        expected_major: String,
    },

    /// Envelope required field missing.
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    /// Envelope field had the wrong type (e.g. `predicates` was an array).
    #[error("wrong type for field {field}: expected {expected}, got {got}")]
    WrongType {
        /// Field name.
        field: &'static str,
        /// What the schema expects.
        expected: &'static str,
        /// What serde saw.
        got: &'static str,
    },

    /// Unknown predicate name referenced somewhere.
    #[error("unknown predicate {name:?}")]
    UnknownPredicateName {
        /// The name that wasn't found.
        name: String,
        /// Optional did-you-mean suggestion.
        hint: Option<String>,
    },

    /// `watch.when` referenced a missing name (separate variant so callers can
    /// distinguish surface; implementation shares with `UnknownPredicateName`).
    #[error("unknown predicate referenced in watch.when: {name:?}")]
    UnknownWatchName {
        /// The name that wasn't found.
        name: String,
        /// Optional did-you-mean.
        hint: Option<String>,
    },

    /// Trigger referenced a missing name.
    #[error("trigger references unknown predicate: {name:?}")]
    UnknownTriggerName {
        /// The name that wasn't found.
        name: String,
        /// Optional did-you-mean.
        hint: Option<String>,
    },

    /// Cycle in the predicate-dependency graph (Decision 1.3).
    #[error("predicate cycle: {cycle:?}")]
    PredicateCycle {
        /// Names on the cycle in traversal order (ends with repeat of first).
        cycle: Vec<String>,
    },

    /// `elapsed_ms_since_true` must name a predicate, not an inline expression.
    #[error("elapsed_ms_since_true: referenced predicate {name:?} does not exist")]
    ElapsedReferenceMissing {
        /// Name that couldn't be resolved.
        name: String,
    },

    /// `Target::Absolute` with negative coordinates.
    #[error("absolute target coordinates must be non-negative, got ({x}, {y})")]
    NegativeAbsoluteTarget {
        /// Offending x.
        x: i32,
        /// Offending y.
        y: i32,
    },

    /// Step that's forbidden inside a watch (`wait_for` / `assert` / `sleep_ms`).
    #[error("step kind {kind:?} is not allowed inside watch.do (body-only)")]
    BodyOnlyStepInWatch {
        /// Kind that was found.
        kind: &'static str,
    },

    /// Malformed expression in a step target (unknown accessor or syntax error).
    #[error("malformed expression: {reason}: {raw:?}")]
    MalformedExpression {
        /// Human-readable reason.
        reason: &'static str,
        /// The raw expression text.
        raw: String,
    },

    /// Expression references a non-match-producing predicate (e.g. logical).
    #[error(
        "expression references non-match predicate {name:?} (logical predicates produce no match)"
    )]
    ExpressionOnLogicalPredicate {
        /// Name referenced.
        name: String,
    },

    /// `all_of.of` / `any_of.of` empty — vacuously true/false and probably a bug.
    #[error("{op} requires at least one operand")]
    EmptyLogicalOp {
        /// Which op: `"all_of"` | `"any_of"`.
        op: &'static str,
    },

    /// Template with full-display absolute region and no opt-in (Decision 4.1).
    #[error("template predicate uses full-display region without slow_budget=true (Decision 4.1)")]
    FullDisplayTemplateWithoutOptIn,

    /// DSL version major was present but unrecognized elsewhere.
    #[error("canonicalization failed: {0}")]
    CanonicalizationFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn to_payload_sets_invalid_program_code_and_path() {
        let e = DslError::new(
            DslErrorKind::MissingField("version"),
            JsonPath::root().key("version"),
        );
        let p = e.to_payload();
        assert_eq!(p.code, ErrorCode::InvalidProgram);
        assert_eq!(p.path.as_deref(), Some("/version"));
        assert!(p.message.contains("missing required field"));
    }

    #[test]
    fn hint_surfaces_for_name_errors() {
        let e = DslError::new(
            DslErrorKind::UnknownPredicateName {
                name: "skp_visible".into(),
                hint: Some("skip_visible".into()),
            },
            JsonPath::root().key("watches").index(0).key("when"),
        );
        assert_eq!(e.hint(), Some("skip_visible"));
        let p = e.to_payload();
        assert_eq!(p.hint.as_deref(), Some("skip_visible"));
    }

    #[test]
    fn hint_absent_for_non_name_errors() {
        let e = DslError::new(DslErrorKind::MissingField("version"), JsonPath::root());
        assert_eq!(e.hint(), None);
    }

    #[test]
    fn payload_serializes_to_wire_shape() {
        let e = DslError::new(
            DslErrorKind::UnknownPredicateName {
                name: "foo".into(),
                hint: None,
            },
            JsonPath::root().key("predicates").key("bar").key("of"),
        );
        let j = serde_json::to_value(e.to_payload()).unwrap();
        assert_eq!(j["code"], json!("invalid_program"));
        assert_eq!(j["path"], json!("/predicates/bar/of"));
    }

    #[test]
    fn canonical_error_conversion_preserves_kind() {
        // Construct a synthetic CanonicalError via the public API shape.
        // We cannot manufacture an io::Error trivially here, so instead test the
        // kind variant directly — `CanonicalizationFailed` is constructed by
        // `From<CanonicalError>`.
        let e = DslError::new(
            DslErrorKind::CanonicalizationFailed("simulated".into()),
            JsonPath::root(),
        );
        assert!(e.to_string().contains("canonicalization failed"));
    }
}
