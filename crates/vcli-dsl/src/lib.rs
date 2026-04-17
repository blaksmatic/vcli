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
