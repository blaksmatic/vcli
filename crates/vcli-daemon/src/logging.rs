//! tracing-subscriber setup with a daily-rolling JSON file + stderr fallback.
//!
//! Returns a `WorkerGuard` the caller must keep alive for the lifetime of the
//! process — dropping it flushes any buffered records. Emits JSON into
//! `<log_dir>/daemon.log.<YYYY-MM-DD>`; `tracing-appender` handles rotation and
//! keeps the rolling logic out of the daemon.

use std::path::Path;

use tracing_appender::rolling;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::error::{DaemonError, DaemonResult};

/// Non-drop-until-shutdown handle to the background log writer thread.
pub struct LogGuard(#[allow(dead_code)] tracing_appender::non_blocking::WorkerGuard);

/// Install the tracing subscriber. Respects `RUST_LOG` via `EnvFilter`; defaults
/// to `info` if unset.
///
/// # Errors
/// Returns [`DaemonError::Logging`] if the subscriber cannot be installed
/// (typically: a subscriber is already set in the current process).
pub fn init(log_dir: &Path) -> DaemonResult<LogGuard> {
    let file_appender = rolling::daily(log_dir, "daemon.log");
    let (nb, guard) = tracing_appender::non_blocking(file_appender);

    let env = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let subscriber = tracing_subscriber::registry().with(env).with(
        fmt::layer()
            .json()
            .with_writer(nb)
            .with_current_span(false)
            .with_span_list(false),
    );

    subscriber
        .try_init()
        .map_err(|e| DaemonError::Logging(format!("{e}")))?;

    Ok(LogGuard(guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Smoke test: constructing the appender does not panic and produces a file
    /// on first log write. We cannot call `init()` from a test (global
    /// subscriber), so this exercises the appender alone.
    #[test]
    fn daily_appender_writes_to_log_dir() {
        use std::io::Write;
        let d = tempdir().unwrap();
        let app = rolling::daily(d.path(), "daemon.log");
        let (mut nb, guard) = tracing_appender::non_blocking(app);
        nb.write_all(b"hello\n").unwrap();
        nb.flush().unwrap();
        drop(guard);
        let mut found = false;
        for entry in std::fs::read_dir(d.path()).unwrap() {
            let e = entry.unwrap();
            if e.file_name().to_string_lossy().starts_with("daemon.log") {
                found = true;
            }
        }
        assert!(found, "expected a daemon.log.* file");
    }
}
