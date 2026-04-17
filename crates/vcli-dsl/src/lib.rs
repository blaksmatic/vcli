//! vcli-dsl — JSON → validated `vcli_core::Program`.
//!
//! This crate is pure: no filesystem access, no asset hashing. Asset path
//! resolution and content hashing belong to `vcli-daemon::submit` per
//! Decision F4 in the v0 spec.
//!
//! Public entry points: [`validate_str`] and [`validate_value`]. Both return
//! the validated program plus a `PredicateHashes` side-table keyed by
//! predicate name so downstream crates can use the hashes as cache keys
//! (Decision 1.3).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod path;
pub use path::JsonPath;

pub mod error;
pub use error::{DslError, DslErrorKind};

pub mod parse;

pub(crate) mod hint;

pub mod predicates;

pub mod expression;
pub use expression::{Accessor, Expression};

pub mod steps;
pub use steps::{validate_body_steps, validate_watch_steps};

pub mod watches;
pub use watches::validate_watches;

pub mod triggers;
pub use triggers::validate_trigger;

pub mod hash;
pub use hash::{compute_predicate_hashes, PredicateHashes};

use vcli_core::Program;

use crate::parse::{parse_envelope_str, parse_envelope_value};
use crate::predicates::validate_predicate_graph;

/// Validated program + predicate hashes side-table.
#[derive(Debug, Clone, PartialEq)]
pub struct Validated {
    /// The parsed `vcli_core::Program`.
    pub program: Program,
    /// Per-name predicate hashes.
    pub hashes: PredicateHashes,
}

/// Validate a JSON string.
///
/// # Errors
///
/// Returns the first [`DslError`] discovered.
pub fn validate_str(src: &str) -> Result<Validated, DslError> {
    let program = parse_envelope_str(src)?;
    finish(program)
}

/// Validate an already-deserialized `serde_json::Value`.
///
/// # Errors
///
/// Returns the first [`DslError`] discovered.
pub fn validate_value(v: &serde_json::Value) -> Result<Validated, DslError> {
    let program = parse_envelope_value(v)?;
    finish(program)
}

fn finish(program: Program) -> Result<Validated, DslError> {
    validate_predicate_graph(&program.predicates)?;
    validate_trigger(&program.trigger, &program.predicates)?;
    validate_watches(&program.watches, &program.predicates)?;
    validate_body_steps(
        &program.body,
        &program.predicates,
        &path::JsonPath::root().key("body"),
    )?;
    let hashes = compute_predicate_hashes(&program.predicates)?;
    Ok(Validated { program, hashes })
}

#[cfg(test)]
mod lib_tests {
    use super::*;
    use serde_json::json;

    fn good_program() -> String {
        json!({
            "version": "0.1",
            "name": "p",
            "trigger": {"kind":"on_submit"},
            "predicates": {
                "skip": {"kind":"template","image":"x.png","confidence":0.9,
                         "region":{"kind":"absolute","box":{"x":0,"y":0,"w":10,"h":10}}}
            },
            "watches": [
                {"when":"skip","do":[{"kind":"click","at":"$skip.match.center"}],
                 "lifetime":{"kind":"persistent"}}
            ],
            "body": []
        })
        .to_string()
    }

    #[test]
    fn validate_str_happy_path_returns_program_and_hashes() {
        let out = validate_str(&good_program()).unwrap();
        assert_eq!(out.program.name, "p");
        assert_eq!(out.hashes.len(), 1);
        assert!(out.hashes.contains_key("skip"));
    }

    #[test]
    fn validate_str_rejects_cycle() {
        let src = json!({
            "version": "0.1",
            "name": "p",
            "trigger": {"kind":"on_submit"},
            "predicates": {
                "a": {"kind":"not","of":"b"},
                "b": {"kind":"not","of":"a"}
            },
            "watches": [],
            "body": []
        })
        .to_string();
        let e = validate_str(&src).unwrap_err();
        assert!(matches!(e.kind, error::DslErrorKind::PredicateCycle { .. }));
    }

    #[test]
    fn validate_str_rejects_unknown_trigger_name() {
        let src = json!({
            "version": "0.1",
            "name": "p",
            "trigger": {"kind":"on_predicate","name":"nope"},
            "predicates": {},
            "watches": [],
            "body": []
        })
        .to_string();
        let e = validate_str(&src).unwrap_err();
        assert!(matches!(
            e.kind,
            error::DslErrorKind::UnknownTriggerName { .. }
        ));
    }

    #[test]
    fn validate_value_and_validate_str_agree() {
        let v: serde_json::Value = serde_json::from_str(&good_program()).unwrap();
        let a = validate_value(&v).unwrap();
        let b = validate_str(&good_program()).unwrap();
        assert_eq!(a.program.name, b.program.name);
        assert_eq!(a.hashes, b.hashes);
    }
}
