//! vcli-perception — Tier-1 and Tier-2 predicate evaluators for vcli.
//!
//! See `docs/superpowers/specs/2026-04-16-vcli-design.md` §"Perception pipeline"
//! and Decision A (DashMap cache, `&self` eval under `par_iter`).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
