//! vcli-cli — user-facing command-line interface for the vcli daemon.
//!
//! See `docs/superpowers/specs/2026-04-16-vcli-design.md` §567–590 for the
//! authoritative CLI surface. This crate is a thin transport+formatting layer
//! over `vcli-ipc`; the daemon does all real work.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod cli;
pub mod error;

pub use cli::{Cli, Command, DaemonCommand, OutputMode, StateFilter};
pub use error::{CliError, CliResult, ExitCode};

/// Entry point used by `src/bin/vcli.rs`. Returns the process exit code
/// (spec §589: `0`, `1`, `2`, `3`, `4`).
#[must_use]
pub fn run() -> i32 {
    ExitCode::Success.into()
}
