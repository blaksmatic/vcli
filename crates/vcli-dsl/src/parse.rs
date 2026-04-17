//! Parse the outer envelope of a program. This step produces a
//! `vcli_core::Program` via `serde_json::from_value` but wraps errors with a
//! path-qualified shape, and gates the DSL major version before any other
//! validation runs (so clearer errors dominate).

use serde_json::Value;

use vcli_core::{DslVersion, Program};

use crate::error::{DslError, DslErrorKind};
use crate::path::JsonPath;

/// Supported DSL major digit.
pub const SUPPORTED_DSL_MAJOR: &str = "0";

/// Parse raw JSON text into a [`Program`] envelope with the version gate applied.
///
/// Errors point at `/version` when the version is wrong, and at the serde
/// location (best-effort) when serde rejects the shape.
///
/// # Errors
///
/// Returns [`DslError`] on any parse or version mismatch.
pub fn parse_envelope_str(src: &str) -> Result<Program, DslError> {
    let v: Value = serde_json::from_str(src)
        .map_err(|e| DslError::new(DslErrorKind::JsonParse(e.to_string()), JsonPath::root()))?;
    parse_envelope_value(&v)
}

/// Parse an already-deserialized `serde_json::Value` envelope with version gate.
///
/// # Errors
///
/// Returns [`DslError`] on version mismatch, missing required fields, or type
/// mismatches detected by serde.
pub fn parse_envelope_value(v: &Value) -> Result<Program, DslError> {
    // Shape check: must be an object before looking for fields.
    if !v.is_object() {
        return Err(DslError::new(
            DslErrorKind::WrongType {
                field: "<root>",
                expected: "object",
                got: type_name(v),
            },
            JsonPath::root(),
        ));
    }

    // Version gate BEFORE serde deserialization so we can report a clean error
    // even if later sections would also fail under a wrong version.
    let ver = v
        .get("version")
        .ok_or_else(|| DslError::new(DslErrorKind::MissingField("version"), JsonPath::root()))?;
    let ver_str = ver.as_str().ok_or_else(|| {
        DslError::new(
            DslErrorKind::WrongType {
                field: "version",
                expected: "string",
                got: type_name(ver),
            },
            JsonPath::root().key("version"),
        )
    })?;
    let dsl_version = DslVersion(ver_str.to_string());
    if dsl_version.major() != SUPPORTED_DSL_MAJOR {
        return Err(DslError::new(
            DslErrorKind::UnsupportedDslVersion {
                found: ver_str.to_string(),
                expected_major: SUPPORTED_DSL_MAJOR.to_string(),
            },
            JsonPath::root().key("version"),
        ));
    }

    // Now let serde do the heavy lifting.
    serde_json::from_value::<Program>(v.clone())
        .map_err(|e| DslError::new(DslErrorKind::JsonParse(e.to_string()), JsonPath::root()))
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DslErrorKind;

    #[test]
    fn minimal_valid_envelope_parses() {
        let src = r#"{
            "version": "0.1",
            "name": "x",
            "trigger": { "kind": "on_submit" },
            "predicates": {},
            "watches": [],
            "body": []
        }"#;
        let p = parse_envelope_str(src).unwrap();
        assert_eq!(p.name, "x");
        assert_eq!(p.version.0, "0.1");
    }

    #[test]
    fn missing_version_reports_missing_field() {
        let src = r#"{ "name": "x", "trigger": {"kind":"on_submit"} }"#;
        let e = parse_envelope_str(src).unwrap_err();
        assert!(matches!(e.kind, DslErrorKind::MissingField("version")));
        assert!(e.path.is_root());
    }

    #[test]
    fn wrong_version_type_reports_wrong_type() {
        let src = r#"{ "version": 1 }"#;
        let e = parse_envelope_str(src).unwrap_err();
        match e.kind {
            DslErrorKind::WrongType {
                field,
                expected,
                got,
            } => {
                assert_eq!(field, "version");
                assert_eq!(expected, "string");
                assert_eq!(got, "number");
            }
            other => panic!("wrong kind: {other:?}"),
        }
        assert_eq!(e.path.to_string(), "/version");
    }

    #[test]
    fn unsupported_major_rejected() {
        let src = r#"{ "version": "1.0", "name": "x", "trigger": {"kind":"on_submit"} }"#;
        let e = parse_envelope_str(src).unwrap_err();
        match e.kind {
            DslErrorKind::UnsupportedDslVersion {
                found,
                expected_major,
            } => {
                assert_eq!(found, "1.0");
                assert_eq!(expected_major, "0");
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn non_object_root_rejected() {
        let e = parse_envelope_str("[]").unwrap_err();
        match e.kind {
            DslErrorKind::WrongType { got, .. } => assert_eq!(got, "array"),
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn serde_shape_error_surfaces_as_json_parse() {
        // trigger.kind is a known tagged enum; garbage value should fail deserialization.
        let src = r#"{
            "version": "0.1",
            "name": "x",
            "trigger": { "kind": "explode" },
            "predicates": {},
            "watches": [],
            "body": []
        }"#;
        let e = parse_envelope_str(src).unwrap_err();
        assert!(matches!(e.kind, DslErrorKind::JsonParse(_)));
    }

    #[test]
    fn bad_json_reports_parse_error() {
        let e = parse_envelope_str("{ not json").unwrap_err();
        assert!(matches!(e.kind, DslErrorKind::JsonParse(_)));
    }
}
