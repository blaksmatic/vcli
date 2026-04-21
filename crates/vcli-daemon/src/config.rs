//! Resolved on-disk layout. One `Config` value, produced at startup, threaded
//! everywhere that needs a path. Every caller is driven by the struct — nothing
//! touches `$HOME` directly so tests stay hermetic.
//!
//! Layout (spec §Persistence → Data layout):
//!   `data_root`  = `~/Library/Application Support/vcli`  (macOS)
//!                | `~/.local/share/vcli`                  (Linux / XDG)
//!   db           = `<data_root>/vcli.db`
//!   pidfile      = `<data_root>/daemon.pid`
//!   socket       = `vcli_ipc::default_socket_path()`
//!   `log_dir`    = `~/Library/Logs/vcli`                  (macOS)
//!                | `~/.cache/vcli/logs`                   (Linux / XDG)
//!   `log_file`   = `<log_dir>/daemon.log` (rotated daily, 7-day retention)

use std::path::{Path, PathBuf};

use vcli_ipc::SocketPath;

use crate::error::{DaemonError, DaemonResult};

/// Every path the daemon needs to know. Built once at startup.
#[derive(Debug, Clone)]
pub struct Config {
    /// Directory holding `vcli.db`, `daemon.pid`, and the assets tree.
    pub data_root: PathBuf,
    /// Resolved socket path + its resolution origin (for `vcli health`).
    pub socket: SocketPath,
    /// Directory for rotating log files.
    pub log_dir: PathBuf,
}

impl Config {
    /// Resolve from the platform defaults. macOS uses Apple locations; Linux uses XDG.
    ///
    /// # Errors
    /// [`DaemonError::Paths`] if `$HOME` (or its platform equivalent) cannot be resolved.
    pub fn from_platform_defaults() -> DaemonResult<Self> {
        let data_root = platform_data_root()?;
        let log_dir = platform_log_dir()?;
        let socket = vcli_ipc::default_socket_path()
            .map_err(|e| DaemonError::Paths(format!("socket path: {e}")))?;
        Ok(Self {
            data_root,
            socket,
            log_dir,
        })
    }

    /// Build a config rooted at an explicit directory — used by tests and
    /// by anyone wiring a non-default install.
    #[must_use]
    pub fn with_roots(
        data_root: impl Into<PathBuf>,
        log_dir: impl Into<PathBuf>,
        socket: SocketPath,
    ) -> Self {
        Self {
            data_root: data_root.into(),
            socket,
            log_dir: log_dir.into(),
        }
    }

    /// Absolute pidfile path (`<data_root>/daemon.pid`).
    #[must_use]
    pub fn pidfile_path(&self) -> PathBuf {
        self.data_root.join("daemon.pid")
    }

    /// Absolute log file path (`<log_dir>/daemon.log`).
    #[must_use]
    pub fn log_file_path(&self) -> PathBuf {
        self.log_dir.join("daemon.log")
    }
}

#[cfg(target_os = "macos")]
fn platform_data_root() -> DaemonResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| DaemonError::Paths("no home dir".into()))?;
    Ok(home
        .join("Library")
        .join("Application Support")
        .join("vcli"))
}

#[cfg(target_os = "macos")]
fn platform_log_dir() -> DaemonResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| DaemonError::Paths("no home dir".into()))?;
    Ok(home.join("Library").join("Logs").join("vcli"))
}

#[cfg(not(target_os = "macos"))]
fn platform_data_root() -> DaemonResult<PathBuf> {
    let d = dirs::data_local_dir().ok_or_else(|| DaemonError::Paths("no XDG_DATA_HOME".into()))?;
    Ok(d.join("vcli"))
}

#[cfg(not(target_os = "macos"))]
fn platform_log_dir() -> DaemonResult<PathBuf> {
    let d = dirs::cache_dir().ok_or_else(|| DaemonError::Paths("no XDG_CACHE_HOME".into()))?;
    Ok(d.join("vcli").join("logs"))
}

/// Ensure every directory in `cfg` exists. Idempotent.
///
/// # Errors
/// IO errors from `fs::create_dir_all`.
pub fn ensure_dirs(cfg: &Config) -> DaemonResult<()> {
    mkdir_p(&cfg.data_root)?;
    mkdir_p(&cfg.log_dir)?;
    Ok(())
}

fn mkdir_p(p: &Path) -> DaemonResult<()> {
    std::fs::create_dir_all(p).map_err(DaemonError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use vcli_ipc::socket_path::SocketPathOrigin;

    fn fake_sock(p: &Path) -> SocketPath {
        SocketPath {
            path: p.to_path_buf(),
            origin: SocketPathOrigin::Override,
        }
    }

    #[test]
    fn with_roots_builds_a_usable_config() {
        let d = tempdir().unwrap();
        let cfg = Config::with_roots(
            d.path().join("data"),
            d.path().join("logs"),
            fake_sock(&d.path().join("vcli.sock")),
        );
        assert_eq!(cfg.pidfile_path(), d.path().join("data").join("daemon.pid"));
        assert_eq!(
            cfg.log_file_path(),
            d.path().join("logs").join("daemon.log")
        );
    }

    #[test]
    fn ensure_dirs_creates_nested_paths() {
        let d = tempdir().unwrap();
        let cfg = Config::with_roots(
            d.path().join("a").join("b"),
            d.path().join("c").join("d"),
            fake_sock(&d.path().join("vcli.sock")),
        );
        ensure_dirs(&cfg).unwrap();
        assert!(cfg.data_root.is_dir());
        assert!(cfg.log_dir.is_dir());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn platform_defaults_use_library_on_macos() {
        let got = platform_data_root().unwrap();
        assert!(got.ends_with("Library/Application Support/vcli"), "{got:?}");
    }
}
