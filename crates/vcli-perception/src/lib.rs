//! vcli-perception — Tier-1 and Tier-2 predicate evaluators for vcli.
//!
//! See `docs/superpowers/specs/2026-04-16-vcli-design.md` §"Perception pipeline"
//! and Decision A (DashMap cache, `&self` eval under `par_iter`).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod error;

pub use error::{PerceptionError, Result};

pub mod evaluator;

pub use evaluator::{EvalCtx, Evaluator};

pub mod cache;
pub mod state;

pub mod frame_view;

pub mod color_at;

pub use color_at::ColorAtEvaluator;

pub mod pixel_diff;

pub use pixel_diff::PixelDiffEvaluator;

pub mod logical;

pub use logical::{AllOfEvaluator, AnyOfEvaluator, NotEvaluator};

pub mod perception;
