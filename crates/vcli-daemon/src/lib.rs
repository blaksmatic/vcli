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

pub mod error;

pub use error::{DaemonError, DaemonResult};

pub mod config;

pub use config::{ensure_dirs, Config};

pub mod pidfile;

pub use pidfile::PidFile;

pub mod logging;

pub use logging::{init as init_logging, LogGuard};

pub mod bridge;

pub use bridge::{new_channels, CommandChannel, SchedulerCommand, EVENT_BROADCAST_CAPACITY};

pub mod persist;

pub use persist::{program_id_of, spawn_event_pump};

pub mod handler;

pub use handler::DaemonHandler;

pub mod startup;

pub use startup::{emit_daemon_started, emit_recovery_events, reload_waiting_programs};

pub mod shutdown;

pub use shutdown::{emit_daemon_stopped, install_signal_handlers};

pub mod factory_mock;

pub mod run;

pub use run::{run_foreground, RuntimeBackends, RuntimeFactory};

/// Build the default `RuntimeBackends` for the platform the daemon was
/// compiled for. On macOS this constructs the real `MacCapture` +
/// `CGEventInputSink` (and starts the kill-switch listener). On every
/// other platform this returns the mock bundle from `factory_mock`.
///
/// The binary entry point in `bin/vcli-daemon.rs` calls this. Tests
/// continue to inject their own factories via `RuntimeFactory`.
///
/// # Errors
///
/// On macOS: any `DaemonError::BackendInit` from the real-backend
/// constructor (typically a TCC permission failure for Screen Recording).
/// On other platforms: never fails.
pub fn build_default_backends() -> error::DaemonResult<run::RuntimeBackends> {
    #[cfg(target_os = "macos")]
    {
        factory_macos::build()
    }
    #[cfg(not(target_os = "macos"))]
    {
        factory_mock::build()
    }
}

#[cfg(target_os = "macos")]
mod factory_macos {
    use crate::error::DaemonResult;
    use crate::run::RuntimeBackends;
    /// Stub — replaced in Task 4 with the real macOS wiring.
    pub fn build() -> DaemonResult<RuntimeBackends> {
        crate::factory_mock::build()
    }
}
