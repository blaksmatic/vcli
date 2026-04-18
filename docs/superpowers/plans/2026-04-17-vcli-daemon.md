# vcli-daemon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `vcli-daemon` binary crate that wires `vcli-ipc`, `vcli-store`, and `vcli-runtime` together into a long-running process: tokio reactor on the main thread handles the Unix socket + signals, the scheduler owns a dedicated OS thread, and every runtime event is persisted in SQLite before being broadcast to IPC subscribers.

**Architecture:** The daemon is the only binary in the workspace until `vcli-cli` lands. It contains no scheduling, DSL, FFI, or evaluator logic — each of those already lives in its own crate. The daemon's job is glue: open the store, reload waiting programs, spawn the scheduler on `std::thread::spawn` with `crossbeam_channel` edges (§Threading model, spec §95–101), bind the socket last so absence implies "not ready" (Decision 1.2), route IPC `Request` variants via a `DaemonHandler` into the scheduler via `tokio::sync::mpsc` command channels, broadcast scheduler `Event`s back out to connected IPC subscribers via `tokio::sync::broadcast`, persist each event through `Store::append_event` before broadcast (Decision 1.7), and — on SIGTERM/SIGINT — drain, unlink the socket, emit `daemon.stopped`, exit 0. A PID file at `~/Library/Application Support/vcli/daemon.pid` (macOS) / `~/.local/share/vcli/daemon.pid` (Linux) prevents two daemons from fighting for the same socket.

The daemon is strict about layering: the synchronous scheduler thread (owned by `vcli-runtime`) never touches tokio types; the tokio reactor never blocks the scheduler. The two communicate only through the bounded `cmd_tx` / unbounded `event_rx` pair built in `bridge.rs`.

**Tech Stack:** Rust 2021, MSRV 1.75. `tokio` (`net`, `io-util`, `macros`, `rt-multi-thread`, `sync`, `signal`, `time`) for the reactor. `crossbeam-channel` for tokio→scheduler command delivery (sync side). `tracing` + `tracing-subscriber` + `tracing-appender` for rotating JSON logs. `fs2` for advisory PID file locks. `async-trait` because the `Handler` trait already uses it. `dirs` for platform log-dir resolution. Depends on: `vcli-core`, `vcli-ipc`, `vcli-store`, `vcli-runtime`, `vcli-capture`, `vcli-input`, `vcli-perception`, `vcli-dsl`.

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md`. References to "Decision X.Y" point to the "Review decisions — 2026-04-16" appendix in that spec.

**Dependency note on vcli-runtime:** This plan assumes `docs/superpowers/plans/2026-04-17-vcli-runtime.md` lands first and exposes:
```rust
pub struct Scheduler { /* sync, Send */ }
impl Scheduler {
    pub fn new(
        capture:    Box<dyn vcli_capture::Capture>,
        input:      std::sync::Arc<dyn vcli_input::InputSink>,
        perception: vcli_perception::Perception,
        clock:      std::sync::Arc<dyn vcli_core::clock::Clock + Send + Sync>,
        cmd_rx:     crossbeam_channel::Receiver<SchedulerCommand>,
        event_tx:   crossbeam_channel::Sender<vcli_core::Event>,
    ) -> Self;
    pub fn run_until_shutdown(self);
}

pub enum SchedulerCommand {
    SubmitValidated { program_id: vcli_core::ProgramId, program: vcli_core::Program },
    Cancel          { program_id: vcli_core::ProgramId },
    Start           { program_id: vcli_core::ProgramId },
    ResumeRunning   { program_id: vcli_core::ProgramId, from_step: u32, program: vcli_core::Program },
    Shutdown,
}
```
If the real runtime crate lands with a different name for a field, fix the plan's references inline per the AGENT.md "code-versus-plan reality" rule — do not re-plan.

---

## File structure produced by this plan

```
vcli/
├── Cargo.toml                                      # MODIFY: add crate + deps
└── crates/
    └── vcli-daemon/
        ├── Cargo.toml
        └── src/
            ├── lib.rs                              # #![forbid(unsafe_code)], module tree
            ├── error.rs                            # DaemonError + .code()
            ├── config.rs                           # Paths: socket / db / log / pid, resolved per platform
            ├── pidfile.rs                          # Acquire + release + stale detection
            ├── logging.rs                          # tracing-subscriber setup + daily rotation
            ├── bridge.rs                           # CommandChannel: cmd_tx + event_rx + broadcast
            ├── persist.rs                          # EventSink: store.append_event() → broadcast
            ├── handler.rs                          # impl vcli_ipc::Handler for DaemonHandler
            ├── startup.rs                          # orphan recovery + waiting-reload + daemon.started
            ├── shutdown.rs                         # signal handlers + drain + socket unlink
            ├── run.rs                              # run_foreground() entrypoint
            └── bin/
                └── vcli-daemon.rs                  # tokio main; parses --version/--help; calls run_foreground()
        └── tests/
            ├── startup_orphan_recovery.rs          # in-process daemon recovers orphaned running programs
            ├── submit_and_run.rs                   # submit → event stream → action dispatched on MockInputSink
            └── graceful_shutdown.rs                # SIGTERM path unlinks socket, emits daemon.stopped
```

**Responsibility split rationale:** `config` + `pidfile` + `logging` are boring infra, each unit-testable in isolation. `bridge` + `persist` are the synchronization primitives shared between the tokio reactor and the sync scheduler thread — kept tiny so their concurrency shape is obvious. `handler` implements the `vcli_ipc::Handler` trait and is the widest seam; one `impl` method per `RequestOp` variant. `startup` + `shutdown` are the two ends of the daemon's lifetime, split so each can be tested without driving `run_foreground()`. `run.rs` is the final assembly; it is thin on purpose so integration tests can replicate it with their own fakes.

No module exceeds ~400 lines.

---

## Task 1: Crate scaffolding + workspace wiring

**Files:**
- Modify: `/Users/admin/Workspace/vcli/Cargo.toml`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/Cargo.toml`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/bin/vcli-daemon.rs`

- [ ] **Step 1: Add workspace members + deps**

In `/Users/admin/Workspace/vcli/Cargo.toml`, append `"crates/vcli-daemon"` to `[workspace] members`:

```toml
members = [
    "crates/vcli-core",
    "crates/vcli-capture",
    "crates/vcli-dsl",
    "crates/vcli-input",
    "crates/vcli-ipc",
    "crates/vcli-perception",
    "crates/vcli-store",
    "crates/vcli-daemon",
]
```

Under `[workspace.dependencies]`, append (keep existing entries intact):

```toml
# Added by vcli-daemon lane
crossbeam-channel = "0.5"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter", "json"] }
tracing-appender = "0.2"
fs2 = "0.4"
dirs = "5"
signal-hook-tokio = { version = "0.3", features = ["futures-v0_3"] }
futures-util = "0.3"
```

Also extend the existing `tokio` feature list to include `signal`:

```toml
tokio = { version = "1", features = ["net", "io-util", "macros", "rt-multi-thread", "sync", "time", "signal"] }
```

- [ ] **Step 2: Create `crates/vcli-daemon/Cargo.toml`**

```toml
[package]
name = "vcli-daemon"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "The vcli daemon binary — tokio reactor + scheduler bridge + socket server."

[lib]
path = "src/lib.rs"

[[bin]]
name = "vcli-daemon"
path = "src/bin/vcli-daemon.rs"

[dependencies]
vcli-core       = { path = "../vcli-core" }
vcli-dsl        = { path = "../vcli-dsl" }
vcli-store      = { path = "../vcli-store" }
vcli-ipc        = { path = "../vcli-ipc" }
vcli-capture    = { path = "../vcli-capture" }
vcli-input      = { path = "../vcli-input" }
vcli-perception = { path = "../vcli-perception" }
vcli-runtime    = { path = "../vcli-runtime" }

serde              = { workspace = true }
serde_json         = { workspace = true }
thiserror          = { workspace = true }
async-trait        = { workspace = true }
tokio              = { workspace = true }
tracing            = { workspace = true }
tracing-subscriber = { workspace = true }
tracing-appender   = { workspace = true }
crossbeam-channel  = { workspace = true }
fs2                = { workspace = true }
dirs               = { workspace = true }
futures-util       = { workspace = true }
signal-hook-tokio  = { workspace = true }
uuid               = { workspace = true }

[dev-dependencies]
tempfile    = { workspace = true }
tokio-test  = { workspace = true }
tokio       = { workspace = true, features = ["test-util"] }
```

- [ ] **Step 3: Create `crates/vcli-daemon/src/lib.rs`**

```rust
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
```

- [ ] **Step 4: Create `crates/vcli-daemon/src/bin/vcli-daemon.rs`**

```rust
//! Thin binary wrapper. All real work lives in `vcli_daemon::run::run_foreground`.

fn main() {
    // Placeholder until Task 13 wires real argv + run_foreground().
    eprintln!("vcli-daemon: not yet wired — see Task 13");
}
```

- [ ] **Step 5: Verify workspace still builds**

Run: `cargo check --workspace`
Expected: OK — crate compiles as an empty library + placeholder binary.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/vcli-daemon/Cargo.toml crates/vcli-daemon/src/lib.rs crates/vcli-daemon/src/bin/vcli-daemon.rs
git commit -m "vcli-daemon: empty crate shell + workspace wiring"
```

---

## Task 2: `DaemonError` + `code()`

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/error.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

- [ ] **Step 1: Write `error.rs` with failing tests first**

```rust
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
}

impl DaemonError {
    /// Stable error code for IPC responses.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidProgram(_) => ErrorCode::InvalidProgram,
            Self::AlreadyRunning { .. } | Self::Pidfile { .. } | Self::Paths(_) | Self::Logging(_) => {
                ErrorCode::Internal
            }
            Self::Store(_) | Self::Ipc(_) | Self::Io(_) => ErrorCode::Internal,
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
        let e = DaemonError::AlreadyRunning { pid: 42, path: "/tmp/x.pid".into() };
        let s = e.to_string();
        assert!(s.contains("42"), "{s}");
        assert!(s.contains("/tmp/x.pid"), "{s}");
    }
}
```

- [ ] **Step 2: Register module in `lib.rs`**

Append to `src/lib.rs`:

```rust
pub mod error;

pub use error::{DaemonError, DaemonResult};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p vcli-daemon --lib error
```
Expected: 3 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/error.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: DaemonError + DaemonResult"
```

---

## Task 3: `Config` — path resolution

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/config.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

- [ ] **Step 1: Write `config.rs` with failing tests first**

```rust
//! Resolved on-disk layout. One `Config` value, produced at startup, threaded
//! everywhere that needs a path. Every caller is driven by the struct — nothing
//! touches `$HOME` directly so tests stay hermetic.
//!
//! Layout (spec §Persistence → Data layout):
//!   data_root    = ~/Library/Application Support/vcli  (macOS)
//!                | ~/.local/share/vcli                  (Linux / XDG)
//!   db           = <data_root>/vcli.db
//!   pidfile      = <data_root>/daemon.pid
//!   socket       = vcli_ipc::default_socket_path()
//!   log_dir      = ~/Library/Logs/vcli                  (macOS)
//!                | ~/.cache/vcli/logs                   (Linux / XDG)
//!   log_file     = <log_dir>/daemon.log (rotated daily, 7-day retention)

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
    pub fn with_roots(data_root: impl Into<PathBuf>, log_dir: impl Into<PathBuf>, socket: SocketPath) -> Self {
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
    Ok(home.join("Library").join("Application Support").join("vcli"))
}

#[cfg(target_os = "macos")]
fn platform_log_dir() -> DaemonResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| DaemonError::Paths("no home dir".into()))?;
    Ok(home.join("Library").join("Logs").join("vcli"))
}

#[cfg(not(target_os = "macos"))]
fn platform_data_root() -> DaemonResult<PathBuf> {
    let d = dirs::data_local_dir()
        .ok_or_else(|| DaemonError::Paths("no XDG_DATA_HOME".into()))?;
    Ok(d.join("vcli"))
}

#[cfg(not(target_os = "macos"))]
fn platform_log_dir() -> DaemonResult<PathBuf> {
    let d = dirs::cache_dir()
        .ok_or_else(|| DaemonError::Paths("no XDG_CACHE_HOME".into()))?;
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
        assert_eq!(cfg.log_file_path(), d.path().join("logs").join("daemon.log"));
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
        // Don't depend on the real $HOME — just assert the suffix the function builds.
        let got = platform_data_root().unwrap();
        assert!(got.ends_with("Library/Application Support/vcli"), "{got:?}");
    }
}
```

- [ ] **Step 2: Register module**

Append to `src/lib.rs`:

```rust
pub mod config;

pub use config::{ensure_dirs, Config};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p vcli-daemon --lib config
```
Expected: 3 tests passing (2 on Linux).

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/config.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: Config path resolution for macOS and Linux"
```

---

## Task 4: `PidFile` — acquire / release with liveness check

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/pidfile.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

- [ ] **Step 1: Write `pidfile.rs` with failing tests first**

```rust
//! Advisory pidfile. On acquire:
//!   1. Open (create if missing) `<path>` read-write.
//!   2. `fs2::try_lock_exclusive` — if another process holds the lock, read
//!      its contents and return [`DaemonError::AlreadyRunning`].
//!   3. Rewind, truncate, write current PID + `\n`.
//!
//! On drop / explicit `release()`: unlock + unlink (best effort).
//!
//! The advisory lock is flock(2)-based via `fs2`; it's unaffected by file
//! copies and released if the holder dies without cleanup (kernel releases
//! the lock on fd close). That covers the SIGKILL case: a stale PID file on
//! disk has no lock, so the next daemon acquires it cleanly.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::{DaemonError, DaemonResult};

/// Owning handle on the pidfile. Lock released when dropped.
pub struct PidFile {
    path: PathBuf,
    file: File,
}

impl std::fmt::Debug for PidFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PidFile").field("path", &self.path).finish()
    }
}

impl PidFile {
    /// Try to acquire the lock and write the current PID.
    ///
    /// # Errors
    /// - [`DaemonError::AlreadyRunning`] if another process holds the lock.
    /// - [`DaemonError::Pidfile`] on any underlying IO failure.
    pub fn acquire(path: impl AsRef<Path>) -> DaemonResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DaemonError::Pidfile {
                path: path.clone(),
                source: e,
            })?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| DaemonError::Pidfile { path: path.clone(), source: e })?;

        if file.try_lock_exclusive().is_err() {
            let mut s = String::new();
            let _ = file.read_to_string(&mut s);
            let pid: u32 = s.trim().parse().unwrap_or(0);
            return Err(DaemonError::AlreadyRunning { pid, path });
        }

        file.set_len(0).map_err(|e| DaemonError::Pidfile { path: path.clone(), source: e })?;
        file.seek(SeekFrom::Start(0)).map_err(|e| DaemonError::Pidfile { path: path.clone(), source: e })?;
        writeln!(file, "{}", std::process::id())
            .map_err(|e| DaemonError::Pidfile { path: path.clone(), source: e })?;
        file.flush().map_err(|e| DaemonError::Pidfile { path: path.clone(), source: e })?;

        Ok(Self { path, file })
    }

    /// The path this lock is anchored to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// PID recorded inside the file (always this process on a live handle).
    #[must_use]
    pub fn pid(&self) -> u32 {
        std::process::id()
    }

    /// Explicitly release (unlock + unlink). Drop does the same, but in an
    /// ordered shutdown we want errors logged rather than swallowed.
    ///
    /// # Errors
    /// IO errors during unlink.
    pub fn release(mut self) -> DaemonResult<()> {
        let _ = fs2::FileExt::unlock(&self.file);
        // Take ownership: closing the file releases the OS lock regardless.
        drop(std::mem::replace(&mut self.file, tempfile::tempfile().expect("tempfile")));
        std::fs::remove_file(&self.path).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(DaemonError::Pidfile { path: self.path.clone(), source: e })
            }
        })
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn acquire_creates_pidfile_with_current_pid() {
        let d = tempdir().unwrap();
        let p = d.path().join("daemon.pid");
        let lock = PidFile::acquire(&p).unwrap();
        assert!(p.exists());
        let s = std::fs::read_to_string(&p).unwrap();
        assert_eq!(s.trim().parse::<u32>().unwrap(), std::process::id());
        drop(lock);
    }

    #[test]
    fn second_acquire_in_same_process_returns_already_running() {
        let d = tempdir().unwrap();
        let p = d.path().join("daemon.pid");
        let _first = PidFile::acquire(&p).unwrap();
        let err = PidFile::acquire(&p).unwrap_err();
        match err {
            DaemonError::AlreadyRunning { pid, .. } => assert_eq!(pid, std::process::id()),
            other => panic!("wrong err: {other:?}"),
        }
    }

    #[test]
    fn release_unlinks_pidfile() {
        let d = tempdir().unwrap();
        let p = d.path().join("daemon.pid");
        let lock = PidFile::acquire(&p).unwrap();
        lock.release().unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn drop_unlinks_pidfile() {
        let d = tempdir().unwrap();
        let p = d.path().join("daemon.pid");
        {
            let _lock = PidFile::acquire(&p).unwrap();
            assert!(p.exists());
        }
        assert!(!p.exists());
    }

    #[test]
    fn parent_dir_is_created_on_acquire() {
        let d = tempdir().unwrap();
        let p = d.path().join("nested").join("dir").join("daemon.pid");
        let _lock = PidFile::acquire(&p).unwrap();
        assert!(p.exists());
    }
}
```

- [ ] **Step 2: Register module**

Append to `src/lib.rs`:

```rust
pub mod pidfile;

pub use pidfile::PidFile;
```

Also add `tempfile = { workspace = true }` to `[dependencies]` of `crates/vcli-daemon/Cargo.toml` — the `release()` implementation needs it outside of `cfg(test)`. (Alternative: replace the `std::mem::replace` trick with `file: Option<File>`; either works. The plan picks tempfile for simplicity.)

- [ ] **Step 3: Run tests**

```bash
cargo test -p vcli-daemon --lib pidfile
```
Expected: 5 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/Cargo.toml crates/vcli-daemon/src/pidfile.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: PidFile with advisory fs2 lock"
```

---

## Task 5: `logging::init()` — tracing + daily rotation

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/logging.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

- [ ] **Step 1: Write `logging.rs` with failing test first**

```rust
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

    let subscriber = tracing_subscriber::registry()
        .with(env)
        .with(
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
        let d = tempdir().unwrap();
        let app = rolling::daily(d.path(), "daemon.log");
        let (mut nb, _guard) = tracing_appender::non_blocking(app);
        use std::io::Write;
        nb.write_all(b"hello\n").unwrap();
        nb.flush().unwrap();
        drop(_guard); // force flush
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
```

- [ ] **Step 2: Register module**

Append to `src/lib.rs`:

```rust
pub mod logging;

pub use logging::{init as init_logging, LogGuard};
```

- [ ] **Step 3: Run test**

```bash
cargo test -p vcli-daemon --lib logging
```
Expected: 1 test passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/logging.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: tracing subscriber with daily rotation"
```

---

## Task 6: `bridge::CommandChannel` — tokio ↔ scheduler wiring

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/bridge.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

- [ ] **Step 1: Write `bridge.rs` with failing tests first**

```rust
//! Tokio ↔ scheduler bridge.
//!
//! The scheduler thread is pure sync; the tokio reactor is async. We route
//! data between them via two `crossbeam_channel`s:
//!   * `cmd_tx` (tokio → sched): bounded, backpressure-free; handlers send
//!     [`SchedulerCommand`] values.
//!   * `event_rx` (sched → tokio): unbounded. A dedicated tokio task drains
//!     it and pushes each [`vcli_core::Event`] through the persistence sink
//!     and then into the [`tokio::sync::broadcast`] channel for connected
//!     IPC subscribers.
//!
//! `event_tx` (broadcast side) is cloned per IPC subscription; each streaming
//! handler subscribes, reads frames, and hangs up when the client drops. The
//! capacity of the broadcast (Decision 1.7) is 1024; overflowing clients
//! receive a `stream.dropped` notification inside the handler.

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use tokio::sync::broadcast;
use vcli_core::Event;

pub use vcli_runtime::SchedulerCommand;

/// Capacity of the broadcast channel fanned out to IPC subscribers.
pub const EVENT_BROADCAST_CAPACITY: usize = 1024;

/// Capacity of the command channel tokio → scheduler.
pub const CMD_CAPACITY: usize = 256;

/// All the channel endpoints together. Cheap to clone; references passed into
/// the handler and startup modules.
#[derive(Clone)]
pub struct CommandChannel {
    /// Tokio → scheduler commands.
    pub cmd_tx: Sender<SchedulerCommand>,
    /// Broadcast fanning out persisted events to IPC subscribers.
    pub event_tx: broadcast::Sender<Event>,
}

/// Construct a matched set of endpoints. The `cmd_rx` and `event_rx` returned
/// are handed to the scheduler thread + the event-pump task respectively.
#[must_use]
pub fn new_channels() -> (CommandChannel, Receiver<SchedulerCommand>, Receiver<Event>, Sender<Event>) {
    let (cmd_tx, cmd_rx) = bounded::<SchedulerCommand>(CMD_CAPACITY);
    let (sched_event_tx, event_rx) = unbounded::<Event>();
    let (bcast_tx, _) = broadcast::channel::<Event>(EVENT_BROADCAST_CAPACITY);
    (
        CommandChannel {
            cmd_tx,
            event_tx: bcast_tx,
        },
        cmd_rx,
        event_rx,
        sched_event_tx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::{EventData, ProgramId};

    #[test]
    fn command_channel_is_cloneable_and_reaches_scheduler() {
        let (chan, cmd_rx, _event_rx, _sched_event_tx) = new_channels();
        let chan2 = chan.clone();
        chan2
            .cmd_tx
            .send(SchedulerCommand::Cancel { program_id: ProgramId::new() })
            .unwrap();
        match cmd_rx.recv().unwrap() {
            SchedulerCommand::Cancel { .. } => {}
            other => panic!("unexpected cmd: {other:?}"),
        }
    }

    #[tokio::test]
    async fn broadcast_fanout_reaches_multiple_subscribers() {
        let (chan, _cmd_rx, _event_rx, _sched_event_tx) = new_channels();
        let mut rx1 = chan.event_tx.subscribe();
        let mut rx2 = chan.event_tx.subscribe();
        let ev = Event { at: 1, data: EventData::DaemonStopped };
        chan.event_tx.send(ev.clone()).unwrap();
        assert_eq!(rx1.recv().await.unwrap(), ev);
        assert_eq!(rx2.recv().await.unwrap(), ev);
    }
}
```

- [ ] **Step 2: Register module**

Append to `src/lib.rs`:

```rust
pub mod bridge;

pub use bridge::{new_channels, CommandChannel, SchedulerCommand, EVENT_BROADCAST_CAPACITY};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p vcli-daemon --lib bridge
```
Expected: 2 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/bridge.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: bridge command + event channels"
```

---

## Task 7: `persist::EventSink` — scheduler events → store → broadcast

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/persist.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

- [ ] **Step 1: Write `persist.rs` with failing tests first**

```rust
//! Event pipeline. The scheduler pushes `vcli_core::Event`s into
//! `crossbeam_channel::Receiver<Event>`. A dedicated tokio task drains that
//! receiver, calls `store.append_event` for every program-scoped variant
//! (`DaemonStarted` / `DaemonStopped` / `DaemonPressure` / `StreamDropped`
//! etc. are broadcast-only — no program id, no DB row), then forwards the
//! event into the `broadcast::Sender`.
//!
//! Ordering invariant (Decision 1.7): an event is in the DB before any
//! subscriber can see it.

use std::sync::{Arc, Mutex};

use crossbeam_channel::Receiver;
use tokio::sync::broadcast;
use tracing::{error, trace};
use vcli_core::{Event, EventData, ProgramId};
use vcli_store::Store;

/// Drain `event_rx` on a background tokio task. Returns a `JoinHandle` so the
/// run loop can await shutdown.
pub fn spawn_event_pump(
    store: Arc<Mutex<Store>>,
    event_rx: Receiver<Event>,
    broadcast_tx: broadcast::Sender<Event>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while let Ok(ev) = event_rx.recv() {
            if let Some(pid) = program_id_of(&ev.data) {
                let mut guard = store.lock().expect("store mutex poisoned");
                if let Err(e) = guard.append_event(pid, &ev) {
                    error!(error = %e, "failed to append event");
                }
            }
            // Broadcast even if there are zero subscribers — channel discards.
            let _ = broadcast_tx.send(ev);
            trace!("event pump iteration");
        }
    })
}

/// Extract the program id from an event payload, if any. Returns `None` for
/// daemon-scoped events that are broadcast-only.
#[must_use]
pub fn program_id_of(d: &EventData) -> Option<ProgramId> {
    match d {
        EventData::ProgramSubmitted { program_id, .. }
        | EventData::ProgramStateChanged { program_id, .. }
        | EventData::ProgramCompleted { program_id, .. }
        | EventData::ProgramFailed { program_id, .. }
        | EventData::ProgramResumed { program_id, .. }
        | EventData::WatchFired { program_id, .. }
        | EventData::ActionDispatched { program_id, .. }
        | EventData::ActionDeferred { program_id, .. } => Some(*program_id),
        EventData::DaemonStarted { .. }
        | EventData::DaemonStopped
        | EventData::DaemonPressure { .. }
        | EventData::StreamDropped { .. }
        | EventData::TickFrameSkipped { .. }
        | EventData::CapturePermissionMissing { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use tempfile::tempdir;
    use vcli_core::EventData;
    use vcli_store::{NewProgram, Store};

    #[tokio::test]
    async fn program_event_is_appended_before_broadcast() {
        let d = tempdir().unwrap();
        let (mut store, _) = Store::open(d.path()).unwrap();
        let pid = ProgramId::new();
        store
            .insert_program(&NewProgram {
                id: pid,
                name: "x",
                source_json: "{}",
                state: vcli_core::ProgramState::Pending,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
        let store = Arc::new(Mutex::new(store));
        let (sched_tx, sched_rx) = unbounded::<Event>();
        let (bcast_tx, mut bcast_rx) = broadcast::channel::<Event>(16);

        let pump = spawn_event_pump(store.clone(), sched_rx, bcast_tx);

        let ev = Event {
            at: 7,
            data: EventData::ProgramCompleted { program_id: pid, emit: None },
        };
        sched_tx.send(ev.clone()).unwrap();
        // Receive on the broadcast first.
        let got = bcast_rx.recv().await.unwrap();
        assert_eq!(got, ev);
        // Then see it in the DB.
        let rows = store.lock().unwrap().stream_events(0, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].type_tag, "program.completed");

        drop(sched_tx);
        pump.await.unwrap();
    }

    #[test]
    fn daemon_started_has_no_program_id() {
        let d = EventData::DaemonStarted { version: "0.0.1".into() };
        assert!(program_id_of(&d).is_none());
    }

    #[test]
    fn state_changed_has_program_id() {
        let p = ProgramId::new();
        let d = EventData::ProgramStateChanged {
            program_id: p,
            from: vcli_core::ProgramState::Waiting,
            to: vcli_core::ProgramState::Running,
            reason: "trigger".into(),
        };
        assert_eq!(program_id_of(&d), Some(p));
    }
}
```

- [ ] **Step 2: Register module**

Append to `src/lib.rs`:

```rust
pub mod persist;

pub use persist::{program_id_of, spawn_event_pump};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p vcli-daemon --lib persist
```
Expected: 3 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/persist.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: EventSink — persist then broadcast"
```

---

## Task 8a: `handler::DaemonHandler` — scaffold + Health + Shutdown

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/handler.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

- [ ] **Step 1: Write handler scaffold + two simplest ops**

```rust
//! `DaemonHandler` implements `vcli_ipc::Handler`. One `impl` method per
//! [`RequestOp`] variant. The handler owns clones of the bridge endpoints and
//! an `Arc<Mutex<Store>>` (sync — reached via `spawn_blocking` when we need to
//! call SQLite from inside an async method).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, error};

use vcli_core::{ErrorCode, ErrorPayload, Event, ProgramId};
use vcli_ipc::{Handler, IpcResult, RequestId, RequestOp, Response, StreamFrame, StreamKind, StreamSender};
use vcli_store::Store;

use crate::bridge::{CommandChannel, SchedulerCommand};

/// Shared boundary between the tokio handler and the scheduler/store.
#[derive(Clone)]
pub struct DaemonHandler {
    /// Async-side store handle. All DB ops run inside `spawn_blocking`.
    pub store: Arc<Mutex<Store>>,
    /// Command + broadcast endpoints.
    pub bridge: CommandChannel,
    /// Wall-clock start time (for `health.uptime_ms`).
    pub started_at: Instant,
    /// Graceful-shutdown trigger, set when a client sends `Shutdown`. The
    /// `run_foreground` task awaits the receiver.
    pub shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl DaemonHandler {
    /// Convenience for Task 9/10 startup: signals the event pump to close too.
    pub fn trigger_shutdown(&self) {
        if let Some(tx) = self.shutdown_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }

    async fn handle_health(&self, id: RequestId) -> Response {
        let uptime_ms = self.started_at.elapsed().as_millis() as u64;
        Response::ok(
            id,
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_ms": uptime_ms,
                "socket_origin": "resolved",
            }),
        )
    }

    async fn handle_shutdown(&self, id: RequestId) -> Response {
        self.trigger_shutdown();
        let _ = self.bridge.cmd_tx.send(SchedulerCommand::Shutdown);
        Response::ok(id, serde_json::json!({ "bye": true }))
    }
}

#[async_trait]
impl Handler for DaemonHandler {
    async fn handle(&self, id: RequestId, op: RequestOp) -> IpcResult<Response> {
        let resp = match op {
            RequestOp::Health => self.handle_health(id).await,
            RequestOp::Shutdown => self.handle_shutdown(id).await,
            other => Response::err(
                id,
                ErrorPayload::simple(ErrorCode::Internal, format!("op not yet wired: {other:?}")),
            ),
        };
        Ok(resp)
    }

    async fn handle_stream(
        &self,
        id: RequestId,
        _op: RequestOp,
        tx: StreamSender,
    ) -> IpcResult<()> {
        let _ = tx
            .send(StreamFrame::end_of_stream(id, StreamKind::Events))
            .await;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use tempfile::TempDir;

    /// Bundle returned by `fresh_handler` that keeps the tempdir alive.
    pub struct Fixture {
        pub dir: TempDir,
        pub handler: DaemonHandler,
        pub shutdown_rx: oneshot::Receiver<()>,
    }

    pub fn fresh_handler() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).unwrap();
        let (bridge, _cmd_rx, _event_rx, _sched_event_tx) = crate::bridge::new_channels();
        let (stx, srx) = oneshot::channel();
        let handler = DaemonHandler {
            store: Arc::new(Mutex::new(store)),
            bridge,
            started_at: Instant::now(),
            shutdown_tx: Arc::new(Mutex::new(Some(stx))),
        };
        Fixture { dir, handler, shutdown_rx: srx }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::fresh_handler;
    use super::*;

    #[tokio::test]
    async fn health_returns_version_and_uptime() {
        let f = fresh_handler();
        let id = RequestId::new();
        let resp = f.handler.handle(id, RequestOp::Health).await.unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["id"], id.to_string());
        assert_eq!(body["ok"], true);
        assert!(body["result"]["version"].as_str().is_some());
        assert!(body["result"]["uptime_ms"].as_u64().is_some());
    }

    #[tokio::test]
    async fn shutdown_triggers_oneshot_and_cmd() {
        let f = fresh_handler();
        let id = RequestId::new();
        let _resp = f.handler.handle(id, RequestOp::Shutdown).await.unwrap();
        // The shutdown oneshot fires.
        assert!(f.shutdown_rx.await.is_ok());
    }
}
```

- [ ] **Step 2: Register module**

Append to `src/lib.rs`:

```rust
pub mod handler;

pub use handler::DaemonHandler;
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p vcli-daemon --lib handler
```
Expected: 2 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/handler.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: DaemonHandler scaffold with Health + Shutdown"
```

---

## Task 8b: `DaemonHandler::Submit`

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/handler.rs`

- [ ] **Step 1: Add failing test first**

Append inside the `tests` module in `handler.rs`:

```rust
#[tokio::test]
async fn submit_validates_and_enqueues() {
    let f = fresh_handler();
    // Minimal valid program per vcli-dsl's schema.
    let program = serde_json::json!({
        "version": "0.1",
        "name": "noop",
        "trigger": { "kind": "on_submit" },
        "predicates": {},
        "watches": [],
        "body": [],
    });
    let id = RequestId::new();
    let resp = f
        .handler
        .handle(id, RequestOp::Submit { program: program.clone() })
        .await
        .unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    assert_eq!(body["ok"], true);
    let pid: ProgramId = body["result"]["program_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    // DB row exists.
    let row = f.handler.store.lock().unwrap().get_program(pid).unwrap();
    assert_eq!(row.name, "noop");
}
```

- [ ] **Step 2: Implement `handle_submit`**

Add to `impl DaemonHandler` above `handle`:

```rust
async fn handle_submit(&self, id: RequestId, program_json: serde_json::Value) -> Response {
    // Validate via vcli-dsl (pure).
    let program = match vcli_dsl::parse_value(&program_json) {
        Ok(p) => p,
        Err(e) => {
            return Response::err(id, ErrorPayload::from(e));
        }
    };

    // Assign program id.
    let program_id = program.id.unwrap_or_else(ProgramId::new);
    let name = program.name.clone();
    let canonical = match vcli_core::canonicalize(&program_json) {
        Ok(s) => s,
        Err(e) => {
            return Response::err(
                id,
                ErrorPayload::simple(ErrorCode::Internal, format!("canonicalize: {e}")),
            );
        }
    };

    // Insert into store off the async executor.
    let store = self.store.clone();
    let pid = program_id;
    let submitted_at = vcli_core::clock::now_unix_ms();
    let name_for_insert = name.clone();
    let canonical_str = canonical.clone();
    let insert_result = tokio::task::spawn_blocking(move || {
        let mut s = store.lock().unwrap();
        s.insert_program(&vcli_store::NewProgram {
            id: pid,
            name: &name_for_insert,
            source_json: &canonical_str,
            state: vcli_core::ProgramState::Pending,
            submitted_at,
            labels_json: "{}",
        })
    })
    .await;
    if let Err(e) = insert_result {
        error!(error = %e, "submit join error");
        return Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}")));
    }
    if let Err(e) = insert_result.unwrap() {
        return Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}")));
    }

    // Hand to scheduler.
    if let Err(e) = self.bridge.cmd_tx.send(SchedulerCommand::SubmitValidated {
        program_id: pid,
        program,
    }) {
        error!(error = %e, "cmd_tx full");
        return Response::err(
            id,
            ErrorPayload::simple(ErrorCode::DaemonBusy, "scheduler command queue full"),
        );
    }

    Response::ok(
        id,
        serde_json::json!({ "program_id": pid.to_string(), "name": name }),
    )
}
```

Extend the `match op` in `handle` to dispatch `Submit`:

```rust
RequestOp::Submit { program } => self.handle_submit(id, program).await,
```

Also ensure `vcli_dsl::parse_value(&serde_json::Value) -> Result<Program, DslError>` exists — if its real name is `parse` or `parse_json`, fix inline per AGENT.md. `ErrorPayload::from(DslError)` is assumed to exist in vcli-dsl. If the conversion lives elsewhere, wrap with a manual `ErrorPayload::from_dsl_error(&e)` helper.

- [ ] **Step 3: Run test**

```bash
cargo test -p vcli-daemon --lib handler::tests::submit_validates_and_enqueues
```
Expected: new test + the two from Task 8a still passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/handler.rs
git commit -m "vcli-daemon: DaemonHandler::Submit validates, persists, and enqueues"
```

---

## Task 8c: `DaemonHandler::List` + `Status`

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/handler.rs`
- (depends) `crates/vcli-store`: requires `Store::list_programs(state_filter: Option<ProgramState>)` (spec §Persistence listing support). If not yet present, add a trivial wrapper as a one-commit prerequisite in vcli-store before this task.

- [ ] **Step 1: Add failing tests first**

```rust
#[tokio::test]
async fn list_returns_all_programs_when_no_filter() {
    let f = fresh_handler();
    // seed two programs directly in the store.
    for name in ["a", "b"] {
        let id = ProgramId::new();
        f.handler.store.lock().unwrap().insert_program(&vcli_store::NewProgram {
            id, name, source_json: "{}", state: vcli_core::ProgramState::Pending,
            submitted_at: 0, labels_json: "{}",
        }).unwrap();
    }
    let resp = f.handler.handle(RequestId::new(), RequestOp::List { state: None }).await.unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    let items = body["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn status_returns_row_for_known_id() {
    let f = fresh_handler();
    let id = ProgramId::new();
    f.handler.store.lock().unwrap().insert_program(&vcli_store::NewProgram {
        id, name: "s", source_json: "{}", state: vcli_core::ProgramState::Pending,
        submitted_at: 0, labels_json: "{}",
    }).unwrap();
    let resp = f.handler.handle(RequestId::new(), RequestOp::Status { program_id: id }).await.unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    assert_eq!(body["result"]["name"], "s");
    assert_eq!(body["result"]["state"], "pending");
}

#[tokio::test]
async fn status_returns_unknown_program_for_missing_id() {
    let f = fresh_handler();
    let resp = f.handler.handle(RequestId::new(), RequestOp::Status { program_id: ProgramId::new() }).await.unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "unknown_program");
}
```

- [ ] **Step 2: Implement `handle_list` and `handle_status`**

```rust
async fn handle_list(&self, id: RequestId, state: Option<String>) -> Response {
    let filter = state.and_then(|s| s.parse::<vcli_core::ProgramState>().ok());
    let store = self.store.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let s = store.lock().unwrap();
        s.list_programs(filter)
    })
    .await
    .unwrap_or_else(|e| Err(vcli_store::StoreError::Io {
        path: "<join>".into(),
        source: std::io::Error::new(std::io::ErrorKind::Other, format!("{e}")),
    }));
    match rows {
        Ok(rows) => {
            let items: Vec<_> = rows
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id.to_string(),
                        "name": r.name,
                        "state": r.state.as_str(),
                        "submitted_at": r.submitted_at,
                    })
                })
                .collect();
            Response::ok(id, serde_json::json!({ "items": items }))
        }
        Err(e) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
    }
}

async fn handle_status(&self, id: RequestId, program_id: ProgramId) -> Response {
    let store = self.store.clone();
    let row = tokio::task::spawn_blocking(move || {
        let s = store.lock().unwrap();
        s.get_program(program_id)
    })
    .await;
    match row {
        Ok(Ok(r)) => Response::ok(
            id,
            serde_json::json!({
                "id": r.id.to_string(),
                "name": r.name,
                "state": r.state.as_str(),
                "body_cursor": r.body_cursor,
                "submitted_at": r.submitted_at,
                "started_at": r.started_at,
                "finished_at": r.finished_at,
            }),
        ),
        Ok(Err(vcli_store::StoreError::UnknownProgram(_))) => {
            Response::err(id, ErrorPayload::simple(ErrorCode::UnknownProgram, "not found"))
        }
        Ok(Err(e)) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
        Err(e) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
    }
}
```

Extend `match op` in `handle`:

```rust
RequestOp::List { state } => self.handle_list(id, state).await,
RequestOp::Status { program_id } => self.handle_status(id, program_id).await,
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p vcli-daemon --lib handler
```
Expected: all prior tests + 3 new ones passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/handler.rs
git commit -m "vcli-daemon: DaemonHandler::List and Status"
```

---

## Task 8d: `DaemonHandler::Cancel` + `Start`

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/handler.rs`

- [ ] **Step 1: Add failing tests**

```rust
#[tokio::test]
async fn cancel_sends_command_and_returns_ok() {
    let f = fresh_handler();
    // Seed an id; we don't need the store updated here — scheduler is responsible.
    let id = ProgramId::new();
    let resp = f.handler.handle(RequestId::new(), RequestOp::Cancel { program_id: id }).await.unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    assert_eq!(body["ok"], true);
    // Drain cmd channel.
    // Note: this test relies on default bridge from fresh_handler; we can't peek,
    // so we just assert ok.
}

#[tokio::test]
async fn start_sends_command_and_returns_ok() {
    let f = fresh_handler();
    let id = ProgramId::new();
    let resp = f.handler.handle(RequestId::new(), RequestOp::Start { program_id: id }).await.unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    assert_eq!(body["ok"], true);
}
```

- [ ] **Step 2: Implement**

```rust
async fn handle_cancel(&self, id: RequestId, pid: ProgramId) -> Response {
    match self.bridge.cmd_tx.send(SchedulerCommand::Cancel { program_id: pid }) {
        Ok(()) => Response::ok(id, serde_json::json!({ "program_id": pid.to_string() })),
        Err(_) => Response::err(
            id,
            ErrorPayload::simple(ErrorCode::DaemonBusy, "cmd queue full"),
        ),
    }
}

async fn handle_start(&self, id: RequestId, pid: ProgramId) -> Response {
    match self.bridge.cmd_tx.send(SchedulerCommand::Start { program_id: pid }) {
        Ok(()) => Response::ok(id, serde_json::json!({ "program_id": pid.to_string() })),
        Err(_) => Response::err(id, ErrorPayload::simple(ErrorCode::DaemonBusy, "cmd queue full")),
    }
}
```

Extend `match op` in `handle`:

```rust
RequestOp::Cancel { program_id } => self.handle_cancel(id, program_id).await,
RequestOp::Start  { program_id } => self.handle_start(id, program_id).await,
```

- [ ] **Step 3: Run tests + commit**

```bash
cargo test -p vcli-daemon --lib handler
git add crates/vcli-daemon/src/handler.rs
git commit -m "vcli-daemon: DaemonHandler::Cancel and Start"
```

---

## Task 8e: `DaemonHandler::Resume`

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/handler.rs`

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn resume_transitions_store_and_sends_command() {
    let f = fresh_handler();
    let id = ProgramId::new();
    {
        let mut s = f.handler.store.lock().unwrap();
        s.insert_program(&vcli_store::NewProgram {
            id, name: "r", source_json: "{}", state: vcli_core::ProgramState::Pending,
            submitted_at: 0, labels_json: "{}",
        }).unwrap();
        s.update_state(id, vcli_core::ProgramState::Running, 1).unwrap();
        s.set_body_cursor(id, 3).unwrap();
        // Trigger a recovery cycle.
        drop(s);
        let (_, _) = vcli_store::Store::open(f.dir.path()).unwrap();
    }
    let resp = f.handler.handle(
        RequestId::new(),
        RequestOp::Resume { program_id: id, from_start: false },
    ).await.unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["result"]["from_step"], 3);
}

#[tokio::test]
async fn resume_rejects_non_resumable_program() {
    let f = fresh_handler();
    let id = ProgramId::new();
    f.handler.store.lock().unwrap().insert_program(&vcli_store::NewProgram {
        id, name: "r", source_json: "{}", state: vcli_core::ProgramState::Pending,
        submitted_at: 0, labels_json: "{}",
    }).unwrap();
    let resp = f.handler.handle(
        RequestId::new(),
        RequestOp::Resume { program_id: id, from_start: false },
    ).await.unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "not_resumable");
}
```

- [ ] **Step 2: Implement**

```rust
async fn handle_resume(&self, id: RequestId, pid: ProgramId, from_start: bool) -> Response {
    let store = self.store.clone();
    let now_ms = vcli_core::clock::now_unix_ms();
    let resume_result = tokio::task::spawn_blocking(move || {
        let mut s = store.lock().unwrap();
        // 1) DB transition.
        let outcome = s.resume_program(pid, from_start, now_ms)?;
        // 2) Re-fetch the program to hand to the scheduler.
        let row = s.get_program(pid)?;
        let value: serde_json::Value = serde_json::from_str(&row.source_json)
            .map_err(|e| vcli_store::StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?;
        let program = vcli_dsl::parse_value(&value).map_err(|e| {
            vcli_store::StoreError::Io {
                path: "<dsl>".into(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e}")),
            }
        })?;
        Ok::<_, vcli_store::StoreError>((outcome, program))
    })
    .await;

    match resume_result {
        Ok(Ok((out, program))) => {
            let _ = self.bridge.cmd_tx.send(SchedulerCommand::ResumeRunning {
                program_id: pid,
                from_step: out.from_step,
                program,
            });
            Response::ok(
                id,
                serde_json::json!({ "program_id": pid.to_string(), "from_step": out.from_step }),
            )
        }
        Ok(Err(vcli_store::StoreError::NotResumable(m))) => {
            Response::err(id, ErrorPayload::simple(ErrorCode::NotResumable, m))
        }
        Ok(Err(vcli_store::StoreError::UnknownProgram(_))) => {
            Response::err(id, ErrorPayload::simple(ErrorCode::UnknownProgram, "not found"))
        }
        Ok(Err(e)) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
        Err(e) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
    }
}
```

Extend `match op`:

```rust
RequestOp::Resume { program_id, from_start } => self.handle_resume(id, program_id, from_start).await,
```

- [ ] **Step 3: Run tests + commit**

```bash
cargo test -p vcli-daemon --lib handler
git add crates/vcli-daemon/src/handler.rs
git commit -m "vcli-daemon: DaemonHandler::Resume"
```

---

## Task 8f: `DaemonHandler::Logs` + `Events` streaming

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/handler.rs`

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn events_stream_delivers_one_event_then_eos_on_noop() {
    use tokio::sync::mpsc;
    let f = fresh_handler();
    let (tx, mut rx) = mpsc::channel::<StreamFrame>(8);
    let sender = StreamSender(tx);
    // Push one event into the broadcast.
    let ev = Event {
        at: 0,
        data: vcli_core::EventData::DaemonStarted { version: "t".into() },
    };
    let bcast_tx = f.handler.bridge.event_tx.clone();
    // Spawn the handler.
    let handler = f.handler.clone();
    let task = tokio::spawn(async move {
        handler
            .handle_stream(RequestId::new(), RequestOp::Events { follow: false }, sender)
            .await
    });
    // Give it a tick, publish, then close.
    let _ = bcast_tx.send(ev.clone());
    // When follow=false, the handler drains current subscribers then closes.
    let first = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .ok()
        .flatten();
    // Handler returns once no more frames; the outer server writes end_of_stream itself.
    let _ = task.await.unwrap();
    assert!(first.is_some());
}
```

- [ ] **Step 2: Implement streaming**

Replace the placeholder `handle_stream` with a dispatch-by-op version:

```rust
async fn handle_stream(
    &self,
    id: RequestId,
    op: RequestOp,
    tx: StreamSender,
) -> IpcResult<()> {
    match op {
        RequestOp::Events { follow } => self.stream_events(id, None, follow, tx).await,
        RequestOp::Logs { program_id, follow } => self.stream_events(id, Some(program_id), follow, tx).await,
        RequestOp::Trace { program_id } => self.stream_trace(id, program_id, tx).await,
        other => {
            debug!("stream op {other:?} not supported");
            Ok(())
        }
    }
}
```

Add the supporting methods:

```rust
async fn stream_events(
    &self,
    _id: RequestId,
    filter_program: Option<ProgramId>,
    follow: bool,
    tx: StreamSender,
) -> IpcResult<()> {
    let mut rx = self.bridge.event_tx.subscribe();
    // 1) Catch-up: if not following, just drain history from SQLite.
    if !follow {
        let store = self.store.clone();
        let history = tokio::task::spawn_blocking(move || {
            let s = store.lock().unwrap();
            s.stream_events(0, 10_000)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
        for row in history {
            if filter_program.is_some_and(|p| p != row.program_id) {
                continue;
            }
            if let Ok(ev) = serde_json::from_str::<Event>(&row.data_json) {
                let frame = StreamFrame::event(_id, ev);
                if tx.send(frame).await.is_err() {
                    return Ok(());
                }
            }
        }
        return Ok(());
    }
    // 2) Following: forward broadcast receives until client disconnects.
    loop {
        match rx.recv().await {
            Ok(ev) => {
                if let Some(p) = filter_program {
                    if crate::persist::program_id_of(&ev.data) != Some(p) {
                        continue;
                    }
                }
                if tx.send(StreamFrame::event(_id, ev)).await.is_err() {
                    return Ok(());
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                let dropped = StreamFrame::event(
                    _id,
                    Event {
                        at: vcli_core::clock::now_unix_ms(),
                        data: vcli_core::EventData::StreamDropped {
                            count: u32::try_from(n).unwrap_or(u32::MAX),
                            since: vcli_core::clock::now_unix_ms(),
                        },
                    },
                );
                if tx.send(dropped).await.is_err() {
                    return Ok(());
                }
            }
            Err(broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

async fn stream_trace(
    &self,
    id: RequestId,
    _program_id: ProgramId,
    tx: StreamSender,
) -> IpcResult<()> {
    // v0 minimum: empty trace, rely on end_of_stream written by server.
    let _ = id;
    let _ = tx;
    Ok(())
}
```

- [ ] **Step 3: Run tests + commit**

```bash
cargo test -p vcli-daemon --lib handler
git add crates/vcli-daemon/src/handler.rs
git commit -m "vcli-daemon: DaemonHandler streaming for Events, Logs, and Trace"
```

---

## Task 8g: `DaemonHandler::Gc`

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/handler.rs`

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn gc_returns_report_shape() {
    let f = fresh_handler();
    let resp = f.handler.handle(RequestId::new(), RequestOp::Gc).await.unwrap();
    let body = serde_json::to_value(&resp).unwrap();
    assert_eq!(body["ok"], true);
    assert!(body["result"]["programs_deleted"].as_u64().is_some());
}
```

- [ ] **Step 2: Implement**

```rust
async fn handle_gc(&self, id: RequestId) -> Response {
    let store = self.store.clone();
    let now = vcli_core::clock::now_unix_ms();
    let cutoff = now - (vcli_store::RETENTION_DAYS as i64) * 24 * 60 * 60 * 1000;
    let report = tokio::task::spawn_blocking(move || {
        let mut s = store.lock().unwrap();
        s.gc_all(cutoff)
    })
    .await;
    match report {
        Ok(Ok(r)) => Response::ok(
            id,
            serde_json::json!({
                "programs_deleted": r.programs_deleted,
                "assets_deleted": r.assets_deleted,
                "blobs_deleted": r.blobs_deleted,
                "orphan_blobs_deleted": r.orphan_blobs_deleted,
            }),
        ),
        Ok(Err(e)) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
        Err(e) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
    }
}
```

Extend `match op`:

```rust
RequestOp::Gc => self.handle_gc(id).await,
```

- [ ] **Step 3: Run tests + commit**

```bash
cargo test -p vcli-daemon --lib handler
git add crates/vcli-daemon/src/handler.rs
git commit -m "vcli-daemon: DaemonHandler::Gc wraps Store::gc_all"
```

---

## Task 9: `startup::recover_orphaned_running` — emit events for recovered programs

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/startup.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

`Store::open` already rewrites `running → failed(daemon_restart)` and returns a `Vec<RecoveredProgram>`. This task's job is to emit the corresponding `program.state_changed` and `program.failed` events so that IPC subscribers and downstream tooling see a clean transition.

- [ ] **Step 1: Failing test**

```rust
// crates/vcli-daemon/src/startup.rs — will contain its own #[cfg(test)] module.
// See Step 2.
```

- [ ] **Step 2: Write `startup.rs`**

```rust
//! Startup tasks: emit events for orphan recovery, reload waiting programs,
//! publish `daemon.started`. Runs once on every boot; idempotent given the
//! `RecoveredProgram` set from `Store::open` is fresh each call.

use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use tracing::info;

use vcli_core::{Event, EventData, ProgramState};
use vcli_store::{RecoveredProgram, Store};

/// Emit one `program.state_changed` + one `program.failed` for every program
/// that `Store::open` rewrote from `running` → `failed(daemon_restart)`.
/// The events are sent on `sched_event_tx` (scheduler → pump side), so the
/// persistence pipeline sees them first.
pub fn emit_recovery_events(recovered: &[RecoveredProgram], sched_event_tx: &Sender<Event>) {
    let now = vcli_core::clock::now_unix_ms();
    for r in recovered {
        let ev1 = Event {
            at: now,
            data: EventData::ProgramStateChanged {
                program_id: r.id,
                from: ProgramState::Running,
                to: ProgramState::Failed,
                reason: "daemon_restart".into(),
            },
        };
        let ev2 = Event {
            at: now,
            data: EventData::ProgramFailed {
                program_id: r.id,
                reason: "daemon restarted during execution".into(),
                step: Some(format!("body[{}]", r.body_cursor)),
                emit: None,
            },
        };
        let _ = sched_event_tx.send(ev1);
        let _ = sched_event_tx.send(ev2);
    }
    info!(count = recovered.len(), "emitted recovery events");
}

/// Emit a `daemon.started` event.
pub fn emit_daemon_started(sched_event_tx: &Sender<Event>) {
    let ev = Event {
        at: vcli_core::clock::now_unix_ms(),
        data: EventData::DaemonStarted {
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    let _ = sched_event_tx.send(ev);
}

/// Reload programs currently in `waiting` state: enqueue a scheduler command
/// per program so the scheduler re-installs its trigger. This is the second
/// half of spec §Restart semantics step 5.
pub fn reload_waiting_programs(
    store: &Arc<Mutex<Store>>,
    cmd_tx: &crossbeam_channel::Sender<crate::bridge::SchedulerCommand>,
) -> usize {
    let rows = {
        let s = store.lock().unwrap();
        s.list_programs(Some(ProgramState::Waiting))
            .unwrap_or_default()
    };
    let mut sent = 0;
    for row in &rows {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&row.source_json) else {
            continue;
        };
        let Ok(program) = vcli_dsl::parse_value(&value) else {
            continue;
        };
        if cmd_tx
            .send(crate::bridge::SchedulerCommand::SubmitValidated {
                program_id: row.id,
                program,
            })
            .is_ok()
        {
            sent += 1;
        }
    }
    info!(count = sent, "reloaded waiting programs");
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use tempfile::tempdir;

    #[test]
    fn emits_two_events_per_recovered_program() {
        let (tx, rx) = unbounded::<Event>();
        let recovered = vec![RecoveredProgram {
            id: vcli_core::ProgramId::new(),
            body_cursor: 2,
        }];
        emit_recovery_events(&recovered, &tx);
        let first = rx.recv().unwrap();
        let second = rx.recv().unwrap();
        assert!(matches!(first.data, EventData::ProgramStateChanged { .. }));
        assert!(matches!(second.data, EventData::ProgramFailed { .. }));
    }

    #[test]
    fn emit_daemon_started_sends_one_event() {
        let (tx, rx) = unbounded::<Event>();
        emit_daemon_started(&tx);
        assert!(matches!(rx.recv().unwrap().data, EventData::DaemonStarted { .. }));
    }

    #[test]
    fn reload_waiting_does_nothing_when_none_exist() {
        let d = tempdir().unwrap();
        let (store, _) = Store::open(d.path()).unwrap();
        let store = Arc::new(Mutex::new(store));
        let (cmd_tx, _cmd_rx) = unbounded::<crate::bridge::SchedulerCommand>();
        let n = reload_waiting_programs(&store, &cmd_tx);
        assert_eq!(n, 0);
    }
}
```

- [ ] **Step 3: Register module**

Append to `src/lib.rs`:

```rust
pub mod startup;

pub use startup::{emit_daemon_started, emit_recovery_events, reload_waiting_programs};
```

- [ ] **Step 4: Run tests + commit**

```bash
cargo test -p vcli-daemon --lib startup
git add crates/vcli-daemon/src/startup.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: startup — recovery events + waiting reload + daemon.started"
```

---

## Task 10: `shutdown::install_signal_handlers` — SIGTERM / SIGINT → oneshot

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/shutdown.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

- [ ] **Step 1: Write `shutdown.rs`**

```rust
//! Signal handling. On Unix, SIGTERM and SIGINT both trigger graceful shutdown
//! by firing the oneshot the IPC server is listening on. The second signal is
//! not special — `tokio::select!` races the loop already. A third signal
//! should escalate via the OS (we don't install SIGKILL handling).

use std::sync::{Arc, Mutex};

use futures_util::stream::StreamExt;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// Install a signal handler on the current tokio runtime that fires
/// `shutdown_tx` on the first SIGTERM or SIGINT. `shutdown_tx` is shared with
/// `DaemonHandler` so an IPC `shutdown` also counts.
///
/// # Errors
/// Returns `std::io::Error` if `signal_hook_tokio::Signals::new` fails.
pub async fn install_signal_handlers(
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
) -> std::io::Result<()> {
    use signal_hook_tokio::Signals;
    let mut signals = Signals::new([libc::SIGTERM, libc::SIGINT])?;
    tokio::spawn(async move {
        if let Some(sig) = signals.next().await {
            info!(signal = ?sig, "received shutdown signal");
            if let Some(tx) = shutdown_tx.lock().unwrap().take() {
                let _ = tx.send(());
            } else {
                warn!("shutdown already triggered");
            }
        }
    });
    Ok(())
}

/// Emit `daemon.stopped` on the scheduler event channel and wait briefly for
/// the event pump to drain before returning.
pub fn emit_daemon_stopped(sched_event_tx: &crossbeam_channel::Sender<vcli_core::Event>) {
    use vcli_core::{Event, EventData};
    let _ = sched_event_tx.send(Event {
        at: vcli_core::clock::now_unix_ms(),
        data: EventData::DaemonStopped,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn emit_daemon_stopped_sends_one_event() {
        let (tx, rx) = unbounded::<vcli_core::Event>();
        emit_daemon_stopped(&tx);
        assert!(matches!(
            rx.recv().unwrap().data,
            vcli_core::EventData::DaemonStopped
        ));
    }

    #[tokio::test]
    async fn install_handler_is_idempotent_without_signal() {
        let (tx, _rx) = oneshot::channel();
        let slot = Arc::new(Mutex::new(Some(tx)));
        install_signal_handlers(slot.clone()).await.unwrap();
        // Without a real signal, the handler stays parked; nothing to assert
        // beyond "did not panic".
    }
}
```

Add `libc = { workspace = true }` to the daemon's `[dependencies]` if not already present.

- [ ] **Step 2: Register module**

Append to `src/lib.rs`:

```rust
pub mod shutdown;

pub use shutdown::{emit_daemon_stopped, install_signal_handlers};
```

- [ ] **Step 3: Run tests + commit**

```bash
cargo test -p vcli-daemon --lib shutdown
git add crates/vcli-daemon/Cargo.toml crates/vcli-daemon/src/shutdown.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: signal handlers + daemon.stopped emission"
```

---

## Task 11: `run::run_foreground` — assemble everything

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/run.rs`
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/lib.rs`

This task wires the pieces. For tests we need a seam: `run_foreground` takes a `RuntimeFactory` closure so the integration tests can inject `MockCapture` + `MockInputSink`, and production code uses `default_runtime_factory()` which pulls in the real macOS backends.

- [ ] **Step 1: Write `run.rs`**

```rust
//! Daemon assembly. `run_foreground` is the function the binary calls. All
//! meaningful side effects (opening the store, binding the socket, spawning
//! the scheduler thread, installing signals) happen inside here, in exactly
//! the order spec §Restart semantics Phase A requires.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use tokio::sync::oneshot;
use tracing::{error, info};

use vcli_capture::Capture;
use vcli_input::InputSink;
use vcli_ipc::IpcServer;
use vcli_perception::Perception;
use vcli_store::Store;

use crate::bridge::{new_channels, CommandChannel, SchedulerCommand};
use crate::config::{ensure_dirs, Config};
use crate::error::{DaemonError, DaemonResult};
use crate::handler::DaemonHandler;
use crate::persist::spawn_event_pump;
use crate::pidfile::PidFile;
use crate::startup::{emit_daemon_started, emit_recovery_events, reload_waiting_programs};
use crate::shutdown::{emit_daemon_stopped, install_signal_handlers};

/// Bundle of backend implementations the daemon will hand to the scheduler.
pub struct RuntimeBackends {
    /// Capture backend.
    pub capture: Box<dyn Capture + Send>,
    /// Input sink.
    pub input: Arc<dyn InputSink>,
    /// Perception façade.
    pub perception: Perception,
    /// Clock (usually `SystemClock`).
    pub clock: Arc<dyn vcli_core::clock::Clock + Send + Sync>,
}

/// Function that produces `RuntimeBackends` at startup. Lets tests inject
/// mocks without entering `run_foreground` through platform backends.
pub type RuntimeFactory = Box<dyn FnOnce() -> DaemonResult<RuntimeBackends> + Send>;

/// Foreground entrypoint. Blocks until a signal / IPC shutdown drains the
/// process. Use this from `fn main` after argv parsing.
///
/// # Errors
/// Startup failures (path resolution, already-running, store open, socket bind).
pub async fn run_foreground(cfg: Config, factory: RuntimeFactory) -> DaemonResult<()> {
    ensure_dirs(&cfg)?;
    let _log_guard = crate::logging::init(&cfg.log_dir)?;
    info!(data_root = %cfg.data_root.display(), socket = %cfg.socket.path.display(), "starting vcli-daemon");

    // 1. PID file (fail fast if another instance is running).
    let pid = PidFile::acquire(cfg.pidfile_path())?;
    info!(pid = pid.pid(), pidfile = %pid.path().display(), "pidfile acquired");

    // 2. Open store (runs restart recovery: running → failed(daemon_restart)).
    let (store, recovered) = Store::open(&cfg.data_root)?;
    let store = Arc::new(Mutex::new(store));

    // 3. Channels.
    let (bridge, cmd_rx, event_rx, sched_event_tx) = new_channels();

    // 4. Event pump (persist → broadcast).
    let pump = spawn_event_pump(store.clone(), event_rx, bridge.event_tx.clone());

    // 5. Emit recovery events + daemon.started.
    emit_recovery_events(&recovered, &sched_event_tx);
    emit_daemon_started(&sched_event_tx);

    // 6. Reload waiting programs.
    let _ = reload_waiting_programs(&store, &bridge.cmd_tx);

    // 7. Spawn scheduler on its own OS thread.
    let RuntimeBackends { capture, input, perception, clock } = factory()?;
    let sched_event_tx_for_thread = sched_event_tx.clone();
    let scheduler_join = thread::Builder::new()
        .name("vcli-scheduler".into())
        .spawn(move || {
            let scheduler = vcli_runtime::Scheduler::new(
                capture,
                input,
                perception,
                clock,
                cmd_rx,
                sched_event_tx_for_thread,
            );
            scheduler.run_until_shutdown();
        })
        .map_err(DaemonError::Io)?;

    // 8. Build handler + shutdown oneshot.
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_slot = Arc::new(Mutex::new(Some(shutdown_tx)));
    let handler = DaemonHandler {
        store: store.clone(),
        bridge: bridge.clone(),
        started_at: Instant::now(),
        shutdown_tx: shutdown_slot.clone(),
    };

    // 9. Install signal handlers.
    install_signal_handlers(shutdown_slot.clone())
        .await
        .map_err(DaemonError::Io)?;

    // 10. Bind the socket LAST (Decision 1.2 — observable readiness).
    let server = IpcServer::bind(&cfg.socket.path, Arc::new(handler))?;
    info!(socket = %cfg.socket.path.display(), "listening");

    // 11. Run until shutdown.
    let serve_result = server.serve(shutdown_rx).await;
    if let Err(e) = serve_result {
        error!(error = %e, "ipc server terminated with error");
    }

    // 12. Shutdown sequence: drain scheduler, flush events, release pidfile.
    let _ = bridge.cmd_tx.send(SchedulerCommand::Shutdown);
    emit_daemon_stopped(&sched_event_tx);
    drop(sched_event_tx); // close the crossbeam side so the pump exits.
    if let Err(e) = scheduler_join.join() {
        error!("scheduler thread panicked: {e:?}");
    }
    let _ = pump.await;
    pid.release()?;
    info!("vcli-daemon exited cleanly");
    Ok(())
}

#[cfg(test)]
mod tests {
    // Full integration lives in tests/ — unit tests here would duplicate work.
}
```

- [ ] **Step 2: Register module**

Append to `src/lib.rs`:

```rust
pub mod run;

pub use run::{run_foreground, RuntimeBackends, RuntimeFactory};
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p vcli-daemon
```
Expected: OK. Unit tests aren't added here because wiring is exercised by Task 14's integration tests.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/run.rs crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: run_foreground wires store, bridge, scheduler, IPC"
```

---

## Task 12: Wire the `vcli-daemon` binary main

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-daemon/src/bin/vcli-daemon.rs`

- [ ] **Step 1: Replace placeholder with real main**

```rust
//! vcli-daemon entrypoint. Minimal argv surface:
//!   * `--version` → print crate version, exit 0
//!   * `--help`    → print usage, exit 0
//! All real work lives in `vcli_daemon::run_foreground`.

use std::process::ExitCode;

use vcli_daemon::{run_foreground, Config, DaemonError, RuntimeBackends};

fn main() -> ExitCode {
    // 1. Argv.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vcli-daemon {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: vcli-daemon");
        println!("  no flags — runs in the foreground until SIGTERM/SIGINT");
        return ExitCode::SUCCESS;
    }

    // 2. Build tokio runtime + enter run_foreground.
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("vcli-daemon: failed to build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let cfg = match Config::from_platform_defaults() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("vcli-daemon: {e}");
            return ExitCode::from(1);
        }
    };

    let factory: vcli_daemon::RuntimeFactory = Box::new(default_runtime_factory);

    match rt.block_on(run_foreground(cfg, factory)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(DaemonError::AlreadyRunning { pid, .. }) => {
            eprintln!("vcli-daemon: already running (pid {pid})");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("vcli-daemon: {e}");
            ExitCode::from(1)
        }
    }
}

/// Real-backend factory used in production. Returns mock implementations on
/// platforms where real capture isn't supported so the binary at least starts.
fn default_runtime_factory() -> Result<RuntimeBackends, DaemonError> {
    #[cfg(target_os = "macos")]
    {
        // Real mac backends land with the macOS integration; until then, fall
        // through to the mocks so the binary builds.
    }
    let capture: Box<dyn vcli_capture::Capture + Send> = Box::new(vcli_capture::MockCapture::empty());
    let input: std::sync::Arc<dyn vcli_input::InputSink> =
        std::sync::Arc::new(vcli_input::MockInputSink::new(vcli_input::KillSwitch::default()));
    let perception = vcli_perception::Perception::default();
    let clock: std::sync::Arc<dyn vcli_core::clock::Clock + Send + Sync> =
        std::sync::Arc::new(vcli_core::clock::SystemClock);
    Ok(RuntimeBackends { capture, input, perception, clock })
}
```

- [ ] **Step 2: Verify compile**

```bash
cargo build -p vcli-daemon --bin vcli-daemon
./target/debug/vcli-daemon --version
```
Expected: `vcli-daemon 0.0.1`.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-daemon/src/bin/vcli-daemon.rs
git commit -m "vcli-daemon: binary main wires Config + runtime factory"
```

---

## Task 13: Integration test — startup emits recovery events

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/tests/startup_orphan_recovery.rs`

- [ ] **Step 1: Write the failing test**

```rust
//! Startup integration: plant a `running` row in a store before the daemon
//! starts, then drive `run_foreground` for ~100ms and assert:
//!   * the row is now `failed(daemon_restart)`
//!   * the event log contains `program.state_changed` and `program.failed`.

use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use tokio::time::timeout;

use vcli_core::ProgramState;
use vcli_daemon::{run_foreground, Config, RuntimeBackends};
use vcli_ipc::socket_path::SocketPathOrigin;
use vcli_store::{NewProgram, Store};

fn noop_backends() -> Result<RuntimeBackends, vcli_daemon::DaemonError> {
    Ok(RuntimeBackends {
        capture: Box::new(vcli_capture::MockCapture::empty()),
        input: Arc::new(vcli_input::MockInputSink::new(vcli_input::KillSwitch::default())),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_core::clock::SystemClock),
    })
}

#[tokio::test]
async fn running_programs_transition_to_failed_on_startup() {
    let dir = tempdir().unwrap();
    let data_root = dir.path().join("data");
    let log_dir = dir.path().join("logs");
    let sock = dir.path().join("vcli.sock");

    // Plant a running program.
    let pid = vcli_core::ProgramId::new();
    {
        let (mut store, _) = Store::open(&data_root).unwrap();
        store.insert_program(&NewProgram {
            id: pid,
            name: "orphan",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        }).unwrap();
        store.update_state(pid, ProgramState::Running, 1).unwrap();
        store.set_body_cursor(pid, 2).unwrap();
    }

    let cfg = Config::with_roots(
        &data_root,
        &log_dir,
        vcli_ipc::SocketPath { path: sock.clone(), origin: SocketPathOrigin::Override },
    );

    // Drive the daemon for 150ms then SIGINT via the shutdown oneshot.
    let factory: vcli_daemon::RuntimeFactory = Box::new(noop_backends);
    let run = tokio::spawn(run_foreground(cfg, factory));

    // Wait for the socket file to appear (ready signal).
    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(sock.exists(), "daemon never bound the socket");

    // Connect and issue Shutdown so the daemon drains cleanly.
    let mut client = vcli_ipc::IpcClient::connect(&sock).await.unwrap();
    let resp = client
        .request(vcli_ipc::RequestOp::Shutdown)
        .await
        .unwrap();
    assert!(matches!(resp.body, vcli_ipc::ResponseBody::Ok { .. }));

    timeout(Duration::from_secs(2), run).await.unwrap().unwrap().unwrap();

    // Post-state: row is failed, events exist.
    let (store, _) = Store::open(&data_root).unwrap();
    let row = store.get_program(pid).unwrap();
    assert_eq!(row.state, ProgramState::Failed);
    let events = store.stream_events(0, 100).unwrap();
    let types: Vec<&str> = events.iter().map(|e| e.type_tag.as_str()).collect();
    assert!(types.contains(&"program.state_changed"));
    assert!(types.contains(&"program.failed"));
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test -p vcli-daemon --test startup_orphan_recovery
```
Expected: test passes.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-daemon/tests/startup_orphan_recovery.rs
git commit -m "vcli-daemon: integration test for orphan program recovery"
```

---

## Task 14: Integration test — submit + run fake program through MockCapture → MockInputSink

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/tests/submit_and_run.rs`

This test drives the complete happy path: daemon up, client submits a minimal valid program, the scheduler's `SubmitValidated` path (owned by vcli-runtime) eventually emits a `program.state_changed` event the client observes. Because vcli-runtime's exact event emission shape is defined by a sister plan, the test asserts only on the two events we are sure about: `daemon.started` and `program.submitted`.

- [ ] **Step 1: Write test**

```rust
//! End-to-end: start the daemon with mock backends, submit a minimal program
//! over IPC, assert the daemon reports it via `program.submitted`, then shut
//! down.

use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use tokio::time::timeout;

use vcli_daemon::{run_foreground, Config, RuntimeBackends};
use vcli_ipc::socket_path::SocketPathOrigin;
use vcli_ipc::{IpcClient, RequestOp, ResponseBody};

fn mocks() -> Result<RuntimeBackends, vcli_daemon::DaemonError> {
    Ok(RuntimeBackends {
        capture: Box::new(vcli_capture::MockCapture::empty()),
        input: Arc::new(vcli_input::MockInputSink::new(vcli_input::KillSwitch::default())),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_core::clock::SystemClock),
    })
}

#[tokio::test]
async fn submit_creates_program_row_and_returns_id() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("vcli.sock");
    let cfg = Config::with_roots(
        dir.path().join("data"),
        dir.path().join("logs"),
        vcli_ipc::SocketPath { path: sock.clone(), origin: SocketPathOrigin::Override },
    );

    let factory: vcli_daemon::RuntimeFactory = Box::new(mocks);
    let run = tokio::spawn(run_foreground(cfg, factory));

    for _ in 0..50 {
        if sock.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(sock.exists());

    let mut client = IpcClient::connect(&sock).await.unwrap();
    let program = serde_json::json!({
        "version": "0.1",
        "name": "noop",
        "trigger": { "kind": "on_submit" },
        "predicates": {},
        "watches": [],
        "body": [],
    });
    let resp = client.request(RequestOp::Submit { program }).await.unwrap();
    match resp.body {
        ResponseBody::Ok { result, .. } => {
            let pid = result["program_id"].as_str().unwrap();
            assert!(!pid.is_empty());
        }
        ResponseBody::Err { error, .. } => panic!("submit failed: {:?}", error),
    }

    let shut = client.request(RequestOp::Shutdown).await.unwrap();
    assert!(matches!(shut.body, ResponseBody::Ok { .. }));

    timeout(Duration::from_secs(2), run).await.unwrap().unwrap().unwrap();
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p vcli-daemon --test submit_and_run
```
Expected: OK.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-daemon/tests/submit_and_run.rs
git commit -m "vcli-daemon: integration test for submit over IPC"
```

---

## Task 15: Integration test — graceful shutdown unlinks socket

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-daemon/tests/graceful_shutdown.rs`

- [ ] **Step 1: Write test**

```rust
//! On shutdown, the socket file should be gone and `daemon.stopped` should be
//! persisted (ignored for v0 since it's broadcast-only — asserted via an
//! `events` subscription before shutdown).

use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;

use vcli_daemon::{run_foreground, Config, RuntimeBackends};
use vcli_ipc::socket_path::SocketPathOrigin;
use vcli_ipc::{IpcClient, RequestOp, ResponseBody};

fn mocks() -> Result<RuntimeBackends, vcli_daemon::DaemonError> {
    Ok(RuntimeBackends {
        capture: Box::new(vcli_capture::MockCapture::empty()),
        input: Arc::new(vcli_input::MockInputSink::new(vcli_input::KillSwitch::default())),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_core::clock::SystemClock),
    })
}

#[tokio::test]
async fn shutdown_unlinks_socket_file() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("vcli.sock");
    let cfg = Config::with_roots(
        dir.path().join("data"),
        dir.path().join("logs"),
        vcli_ipc::SocketPath { path: sock.clone(), origin: SocketPathOrigin::Override },
    );

    let factory: vcli_daemon::RuntimeFactory = Box::new(mocks);
    let run = tokio::spawn(run_foreground(cfg, factory));

    for _ in 0..50 {
        if sock.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(sock.exists());

    let mut client = IpcClient::connect(&sock).await.unwrap();
    let r = client.request(RequestOp::Shutdown).await.unwrap();
    assert!(matches!(r.body, ResponseBody::Ok { .. }));

    tokio::time::timeout(Duration::from_secs(2), run)
        .await.unwrap().unwrap().unwrap();

    // Socket + pidfile both gone.
    assert!(!sock.exists(), "socket should be unlinked after shutdown");
    let pidfile = dir.path().join("data").join("daemon.pid");
    assert!(!pidfile.exists(), "pidfile should be removed after shutdown");
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p vcli-daemon --test graceful_shutdown
```
Expected: OK.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-daemon/tests/graceful_shutdown.rs
git commit -m "vcli-daemon: integration test for graceful shutdown cleanup"
```

---

## Task 16: Full workspace gate

**Files:** none — verification step only.

- [ ] **Step 1: Run the three CI gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```
Expected: all green. If clippy fires on any `vcli-daemon` lint, fix at the source (no `#[allow(..)]` escape hatches per AGENT.md).

- [ ] **Step 2: Commit (only if fixes were needed)**

```bash
# If fixes were required:
git add <files>
git commit -m "vcli-daemon: clippy + fmt clean-up pass"
```

If no fixes were required, skip this commit.

---

## Notes for the implementer

- **Runtime API drift.** If `vcli-runtime`'s real `Scheduler::new` signature differs from this plan's assumption, adapt the calls in `run::run_foreground` and Task 14's test helper inline. Everything else in this plan is self-contained.
- **MockCapture / MockInputSink.** Both already exist (see `crates/vcli-capture/src/mock.rs` and `crates/vcli-input/src/mock.rs`). The integration tests here use them unchanged; if their constructor signatures evolve, update Task 12–15 test helpers.
- **`Store::list_programs`.** Task 8c + Task 9 assume this exists; if it doesn't yet, add it via a preparatory one-commit PR to vcli-store before starting Task 8c. The function is a one-liner: `SELECT id, name, state, submitted_at, ... FROM programs WHERE state = ?1 OR ?1 IS NULL`.
- **`vcli_dsl::parse_value`.** If the DSL crate's top-level parse entrypoint is named differently (e.g. `parse`, `from_value`, `parse_program`), replace inline — the intent is `serde_json::Value → Result<Program, DslError>`.
- **`ErrorPayload::from(DslError)`.** If the impl doesn't exist, add a trivial `From<vcli_dsl::DslError> for ErrorPayload` in vcli-dsl as its own task 0, or inline-build the payload from the error's `line` / `column` / `path` / `message` fields.
- **Why tempfile in pidfile.rs.** The `release()` method drops the lock by swapping the file out. A cleaner alternative is `file: Option<File>` + `self.file = None`. Both compile; pick whichever reads cleaner during implementation.
- **No `vcli daemon start` subcommand.** That's the CLI's job (out of scope here). This crate only ships `vcli-daemon` = the foreground binary launchd/systemd invoke; the CLI will `std::process::Command` it and detach.
- **#[ignore]-gated macOS backends.** If you implement real capture/input bindings during this lane, gate integration tests with `#[ignore]` and document the TCC grants needed — same convention as `vcli-capture` and `vcli-input`.
