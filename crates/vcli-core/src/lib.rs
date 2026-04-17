//! vcli-core — shared types, canonical JSON, clock abstraction, event taxonomy.
//!
//! See the v0 design spec at `docs/superpowers/specs/2026-04-16-vcli-design.md`
//! for the authoritative definitions implemented here.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod geom;

pub mod ids;

pub use ids::ProgramId;

pub mod frame;

pub use frame::{Frame, FrameFormat};

pub mod clock;

pub use clock::{Clock, SystemClock, TestClock, UnixMs};

pub mod region;

pub use region::{Anchor, Region, WindowIndex};

pub mod predicate;

pub use predicate::{MatchData, Predicate, PredicateKind, PredicateResult};

pub mod action;

pub use action::{Button, InputAction, Modifier};

pub mod step;

pub use step::{OnFail, OnTimeout, Step, Target};
