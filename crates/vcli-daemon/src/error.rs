//! Top-level daemon error. Maps to `vcli_core::ErrorCode` at the IPC boundary.

use std::io;
use std::path::PathBuf;

use thiserror::Error;
use vcli_core::ErrorCode;

/// Convenience alias.
pub type DaemonResult<T> = Result<T, DaemonError>;

/// Errors the daemon produces outside the per-request handler path.
#[derive(Debug, Error)]
pub enum DaemonError {
    /// PID file already held by a live process.
    #[error("another vcli-daemon is already running (pid {pid}, lockfile {path})")]
    AlreadyRunning {
        /// Owning PID.
        pid: u32,
        /// Absolute pidfile path.
        path: PathBuf,
    },

    /// Could not acquire / write the pidfile.
    #[error("pidfile {path}: {source}")]
    Pidfile {
        /// Absolute path.
        path: PathBuf,
        /// Underlying IO cause.
        #[source]
        source: io::Error,
    },

    /// Could not resolve the daemon's data root, socket path, or log dir.
    #[error("path resolution: {0}")]
    Paths(String),

    /// Tracing subscriber failed to install.
    #[error("logging init: {0}")]
    Logging(String),

    /// Store open / migrate failed.
    #[error("store: {0}")]
    Store(#[from] vcli_store::StoreError),

    /// IPC transport setup / serve failure.
    #[error("ipc: {0}")]
    Ipc(#[from] vcli_ipc::IpcError),

    /// Generic IO error during startup / shutdown.
    #[error("io: {0}")]
    Io(#[from] io::Error),

    /// DSL validation of a submitted program failed before scheduler touched it.
    #[error("invalid program: {0}")]
    InvalidProgram(String),

    /// A backend (capture, input, perception, clock) failed to construct
    /// at startup. The daemon refuses to boot rather than enter a
    /// permanently-failing tick loop. See spec Decision B5.
    #[error("{backend} backend init failed: {reason}")]
    BackendInit {
        /// Short backend name: "capture", "input", "perception", "clock".
        backend: &'static str,
        /// Human-readable cause, including remediation hint when known
        /// (e.g., "grant Screen Recording in System Settings → Privacy & Security").
        reason: String,
    },
}

impl DaemonError {
    /// Stable error code for IPC responses.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidProgram(_) => ErrorCode::InvalidProgram,
            Self::AlreadyRunning { .. }
            | Self::Pidfile { .. }
            | Self::Paths(_)
            | Self::Logging(_)
            | Self::Store(_)
            | Self::Ipc(_)
            | Self::Io(_)
            | Self::BackendInit { .. } => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_program_maps_to_invalid_program_code() {
        let e = DaemonError::InvalidProgram("bad path".into());
        assert_eq!(e.code(), ErrorCode::InvalidProgram);
    }

    #[test]
    fn paths_maps_to_internal() {
        let e = DaemonError::Paths("no home".into());
        assert_eq!(e.code(), ErrorCode::Internal);
    }

    #[test]
    fn already_running_display_contains_pid_and_path() {
        let e = DaemonError::AlreadyRunning {
            pid: 42,
            path: "/tmp/x.pid".into(),
        };
        let s = e.to_string();
        assert!(s.contains("42"), "{s}");
        assert!(s.contains("/tmp/x.pid"), "{s}");
    }

    #[test]
    fn backend_init_renders_message_and_maps_to_internal_code() {
        let e = DaemonError::BackendInit {
            backend: "capture",
            reason: "Screen Recording not granted (TCC PermissionDenied)".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("capture"), "msg: {msg}");
        assert!(msg.contains("Screen Recording"), "msg: {msg}");
        assert_eq!(e.code(), ErrorCode::Internal);
    }
}
