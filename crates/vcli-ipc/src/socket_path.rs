//! Socket path resolution per Decision 2.5.
//!
//! macOS → `$TMPDIR/vcli-$UID.sock`
//! Linux → `$XDG_RUNTIME_DIR/vcli.sock` → `/run/user/$UID/vcli.sock` → `/tmp/vcli-$UID.sock`
//!
//! The daemon creates the socket at the same path; CLI discovers via this same
//! resolution, so a mismatch is a bug, not a config hazard.

use std::env;
use std::path::{Path, PathBuf};

use crate::error::{IpcError, IpcResult};

/// Resolved socket path plus the lookup strategy that found it (for diagnostics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPath {
    /// Concrete filesystem path to the socket.
    pub path: PathBuf,
    /// Which strategy produced this path. Useful in `vcli health`.
    pub origin: SocketPathOrigin,
}

/// Which resolution branch produced a `SocketPath`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketPathOrigin {
    /// Explicit override (env var or API arg).
    Override,
    /// macOS `$TMPDIR` branch.
    MacTmpdir,
    /// Linux `$XDG_RUNTIME_DIR` branch.
    XdgRuntimeDir,
    /// Linux `/run/user/$UID` fallback.
    RunUser,
    /// `/tmp/vcli-$UID.sock` fallback (any Unix).
    TmpFallback,
}

/// Environment variable that, when set, overrides all other resolution.
pub const OVERRIDE_ENV: &str = "VCLI_SOCKET";

/// Resolve the default socket path. Returns `Err(IpcError::SocketPath)` if
/// nothing in the fallback chain is usable.
pub fn default_socket_path() -> IpcResult<SocketPath> {
    if let Some(p) = env::var_os(OVERRIDE_ENV) {
        return Ok(SocketPath { path: PathBuf::from(p), origin: SocketPathOrigin::Override });
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(p) = try_mac_tmpdir() {
            return Ok(SocketPath { path: p, origin: SocketPathOrigin::MacTmpdir });
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(p) = try_xdg_runtime_dir() {
            return Ok(SocketPath { path: p, origin: SocketPathOrigin::XdgRuntimeDir });
        }
        if let Some(p) = try_run_user() {
            return Ok(SocketPath { path: p, origin: SocketPathOrigin::RunUser });
        }
    }
    if let Some(p) = try_tmp_fallback() {
        return Ok(SocketPath { path: p, origin: SocketPathOrigin::TmpFallback });
    }
    Err(IpcError::SocketPath(
        "no usable socket directory (tried $VCLI_SOCKET, $TMPDIR, $XDG_RUNTIME_DIR, /run/user, /tmp)"
            .into(),
    ))
}

fn uid() -> u32 {
    // Use `id -u` to obtain the real UID without an unsafe block (required by
    // `#![forbid(unsafe_code)]`). Deviation from plan's unsafe getuid() — same
    // observable behaviour; cached at first call via OnceLock so the fork cost
    // is paid once per process.
    use std::sync::OnceLock;
    static UID: OnceLock<u32> = OnceLock::new();
    *UID.get_or_init(|| {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    })
}

#[cfg(target_os = "macos")]
fn try_mac_tmpdir() -> Option<PathBuf> {
    let d = env::var_os("TMPDIR")?;
    sock_in_dir(Path::new(&d))
}

#[cfg(target_os = "linux")]
fn try_xdg_runtime_dir() -> Option<PathBuf> {
    let d = env::var_os("XDG_RUNTIME_DIR")?;
    let dir = Path::new(&d);
    if dir.as_os_str().is_empty() {
        return None;
    }
    Some(dir.join("vcli.sock"))
}

#[cfg(target_os = "linux")]
fn try_run_user() -> Option<PathBuf> {
    let p = PathBuf::from(format!("/run/user/{}", uid()));
    if p.is_dir() {
        Some(p.join("vcli.sock"))
    } else {
        None
    }
}

fn try_tmp_fallback() -> Option<PathBuf> {
    Some(PathBuf::from(format!("/tmp/vcli-{}.sock", uid())))
}

fn sock_in_dir(dir: &Path) -> Option<PathBuf> {
    if dir.as_os_str().is_empty() {
        return None;
    }
    Some(dir.join(format!("vcli-{}.sock", uid())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard env vars around a test — restores prior values on drop.
    struct EnvGuard {
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(keys: &[(&str, Option<&str>)]) -> Self {
            let saved = keys
                .iter()
                .map(|(k, _)| ((*k).to_string(), env::var_os(k)))
                .collect::<Vec<_>>();
            for (k, v) in keys {
                match v {
                    Some(v) => env::set_var(k, v),
                    None => env::remove_var(k),
                }
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in self.saved.drain(..) {
                match v {
                    Some(v) => env::set_var(&k, v),
                    None => env::remove_var(&k),
                }
            }
        }
    }

    #[test]
    fn override_env_wins_over_all_other_branches() {
        let _g = EnvGuard::set(&[(OVERRIDE_ENV, Some("/custom/vcli.sock"))]);
        let p = default_socket_path().unwrap();
        assert_eq!(p.path, PathBuf::from("/custom/vcli.sock"));
        assert_eq!(p.origin, SocketPathOrigin::Override);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn mac_uses_tmpdir_branch() {
        let _g = EnvGuard::set(&[(OVERRIDE_ENV, None), ("TMPDIR", Some("/private/var/folders/xx"))]);
        let p = default_socket_path().unwrap();
        assert_eq!(p.origin, SocketPathOrigin::MacTmpdir);
        assert!(p.path.starts_with("/private/var/folders/xx"));
        assert!(p.path.to_string_lossy().contains("vcli-"));
        assert!(p.path.extension().unwrap() == "sock");
    }

    #[test]
    fn tmp_fallback_produces_uid_scoped_name() {
        let p = try_tmp_fallback().unwrap();
        let s = p.to_string_lossy();
        assert!(s.starts_with("/tmp/vcli-"));
        assert!(s.ends_with(".sock"));
    }
}
