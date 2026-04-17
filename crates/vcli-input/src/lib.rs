//! vcli-input — input synthesis for vcli.
//!
//! Exposes a synchronous [`InputSink`] trait with a macOS CGEvent backend, a
//! recording [`MockInputSink`] used by downstream crates in tests, and a
//! process-global [`KillSwitch`] (Codex Decision B) that short-circuits every
//! method when a human has signalled STOP via the OS-level chord.
//!
//! Windows is a stub (`unimplemented!()`). See the v0 spec §Input synthesis
//! and §Action confirmation for the contract this crate implements.

#![forbid(unsafe_code)] // relaxed inside macos/cg_events.rs via targeted allow
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod error;

pub use error::InputError;
