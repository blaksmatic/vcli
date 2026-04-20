//! vcli-daemon — the binary that wires IPC, the store, and the scheduler.
//!
//! See the v0 design spec at `docs/superpowers/specs/2026-04-16-vcli-design.md`
//! §Threading model, §IPC, and §Restart semantics for the authoritative behavior.
//!
//! The library surface exists so that the binary in `src/bin/vcli-daemon.rs` is
//! a thin `main`, and so that integration tests can exercise the assembly with
//! mock capture + input backends.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
