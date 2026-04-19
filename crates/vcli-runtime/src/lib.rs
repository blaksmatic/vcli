//! vcli-runtime — deterministic 10 fps scheduler.
//!
//! See the v0 design spec at `docs/superpowers/specs/2026-04-16-vcli-design.md`
//! — §Runtime & scheduler (§311) is the authoritative source for everything
//! in this crate.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

mod arbiter;
mod body;
pub mod clock;
mod command;
mod error;
mod event;
mod expr;
mod program;
mod scheduler;
mod triggers;
mod watches;

pub use clock::{ManualClock, RuntimeClock, SystemRuntimeClock};
pub use command::SchedulerCommand;
pub use error::{ErrorCode, RuntimeError};
pub use scheduler::{Scheduler, SchedulerConfig};
