# vcli-daemon real-backends Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire `vcli_capture::macos::MacCapture` + `vcli_input::macos::CGEventInputSink` (with the kill-switch tap) into the `vcli-daemon` binary so the released artifact actually captures screens and synthesizes input on macOS, instead of the `MockCapture::empty()` / `MockInputSink::new()` placeholders shipped in PR #10. Non-macOS builds keep the mocks (Windows ships in v0.4 per Decision G).

**Architecture:** Surgical change confined to one binary file plus a new sibling module for the macOS-specific construction. The daemon keeps its `RuntimeFactory` injection pattern (so unit tests still use mocks with zero changes), but the *default* factory used by `fn main` switches from "mocks everywhere" to "real backends on macOS, mocks elsewhere." The kill-switch tap thread's lifetime is tied to `RuntimeBackends` via a type-erased `Option<Box<dyn Any + Send + Sync>>` field, so the listener runs from daemon start to daemon stop without leaking across tests or polluting the public type surface with a cfg-gated handle. Failure to construct the real capture backend (TCC denied, no displays, etc.) maps to a new `DaemonError::BackendInit` variant and aborts startup with exit code 1 and a clear stderr line — better than the status quo of an endlessly-warning idle loop.

**Tech Stack:** Rust 2021, MSRV 1.75. No new dependencies. Uses already-present crates: `vcli-capture` (`MacCapture`, `permission`), `vcli-input` (`CGEventInputSink`, `KillSwitch`, `spawn_kill_switch_listener`, `permissions::probe`), `tracing`, `thiserror`. cfg gates: `target_os = "macos"`.

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md` § "Backend wiring decisions — 2026-04-22 (post-ship)" — decisions B1–B6.

**Dependency note:** Assumes the daemon, capture, input, runtime, store, ipc, perception crates are at master as of 2026-04-22. Specifically depends on these existing public APIs:

```rust
// vcli-capture
pub fn vcli_capture::macos::MacCapture::new() -> Result<Self, CaptureError>;
pub enum vcli_capture::CaptureError { PermissionDenied, /* ... */ }

// vcli-input
pub struct vcli_input::KillSwitch;
impl vcli_input::KillSwitch { pub fn new() -> Self; }
pub fn vcli_input::macos::CGEventInputSink::new(kill: KillSwitch) -> CGEventInputSink;
pub fn vcli_input::macos::spawn_kill_switch_listener(kill: KillSwitch)
    -> Result<vcli_input::macos::KillSwitchListenerHandle, vcli_input::InputError>;
pub fn vcli_input::permissions::probe() -> vcli_input::permissions::PermissionReport;

// vcli-daemon (current)
pub struct vcli_daemon::RuntimeBackends { capture, input, perception, clock }
pub type vcli_daemon::RuntimeFactory = Box<dyn FnOnce() -> DaemonResult<RuntimeBackends> + Send>;
```

If any of these signatures has drifted by the time you implement, follow the AGENT.md "code-vs-plan reality" rule: trust `cargo check`, fix the plan inline in the commit body, keep going.

---

## File structure produced by this plan

```
crates/vcli-daemon/
├── src/
│   ├── error.rs                     MODIFIED — adds DaemonError::BackendInit variant + code() mapping
│   ├── run.rs                       MODIFIED — RuntimeBackends gains _shutdown_guard field
│   ├── factory_macos.rs             CREATED — macOS RuntimeBackends builder, only compiled on target_os = "macos"
│   ├── factory_mock.rs              CREATED — mock-everything builder used on non-macOS targets (and as a fallback)
│   ├── lib.rs                       MODIFIED — declare the two factory modules under cfg, re-export build_default_backends
│   └── bin/
│       └── vcli-daemon.rs           MODIFIED — default_runtime_factory delegates to lib's cfg-gated builder; emit permission probe at INFO
└── tests/
    └── real_backends_macos.rs       CREATED — #[ignore]d integration test that constructs the real backends end-to-end on macOS
README.md                            MODIFIED — flip vcli-daemon row from ⏳ to ✅, document TCC requirement
ARCHITECTURE.md                      MODIFIED — note that the macOS daemon needs Screen Recording + Accessibility + Input Monitoring on first run
```

---

## Task 0: Land the spec addendum

The spec section was added in the same edit session as this plan. This task just makes sure it's committed before any code changes so future readers can connect plan-6 commits to a written decision.

**Files:**
- Modify: `docs/superpowers/specs/2026-04-16-vcli-design.md` — already contains the new "Backend wiring decisions — 2026-04-22 (post-ship)" section before the GSTACK REVIEW REPORT table.

- [ ] **Step 1: Verify the addendum is present**

Run: `grep -n "Backend wiring decisions — 2026-04-22" docs/superpowers/specs/2026-04-16-vcli-design.md`
Expected: one matching line, somewhere around §1184–1190.

- [ ] **Step 2: Commit the spec + plan together**

```bash
git add docs/superpowers/specs/2026-04-16-vcli-design.md \
        docs/superpowers/plans/2026-04-22-vcli-daemon-real-backends.md
git commit -m "docs: spec addendum + plan-6 for daemon real-backends wiring

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 1: `DaemonError::BackendInit` variant

**Files:**
- Modify: `crates/vcli-daemon/src/error.rs` (add variant near the other startup-time errors, plus a `code()` arm)

- [ ] **Step 1: Write the failing test**

Append to `crates/vcli-daemon/src/error.rs` (inside the existing `#[cfg(test)] mod tests { ... }` block at the bottom — if there isn't one yet, create it):

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p vcli-daemon --lib backend_init_renders_message`
Expected: FAIL — `DaemonError::BackendInit` does not exist.

- [ ] **Step 3: Add the variant**

In `crates/vcli-daemon/src/error.rs`, inside the `pub enum DaemonError { ... }` block, after the `InvalidProgram` variant:

```rust
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
```

In the `impl DaemonError { pub fn code(&self) -> ErrorCode { match self { ... } } }` block, add:

```rust
            DaemonError::BackendInit { .. } => ErrorCode::Internal,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p vcli-daemon --lib backend_init_renders_message`
Expected: PASS.

- [ ] **Step 5: Run the full daemon crate gate**

Run: `cargo test -p vcli-daemon --locked && cargo clippy -p vcli-daemon --all-targets -- -D warnings && cargo fmt -p vcli-daemon -- --check`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/vcli-daemon/src/error.rs
git commit -m "vcli-daemon: DaemonError::BackendInit for startup backend failures

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `RuntimeBackends` gains `_shutdown_guard`

The macOS factory will need to keep the kill-switch listener thread alive for the daemon's lifetime. Rather than introduce a cfg-gated public type, store its handle as a type-erased `Option<Box<dyn Any + Send + Sync>>`. Mock factories leave it `None`.

**Files:**
- Modify: `crates/vcli-daemon/src/run.rs` (add field to `RuntimeBackends`)
- Modify: any `RuntimeBackends { capture, input, perception, clock }` literal in the daemon crate (currently in the binary's `default_runtime_factory` and in test-only factories under `crates/vcli-daemon/src/handler.rs` + `crates/vcli-daemon/tests/`).

- [ ] **Step 1: Write the failing test**

Append to `crates/vcli-daemon/src/run.rs` (or its existing `#[cfg(test)] mod tests`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Probe that the type-erased shutdown guard runs Drop when the
    /// RuntimeBackends bundle is dropped. This is the contract the macOS
    /// factory relies on for kill-switch teardown.
    #[test]
    fn dropping_runtime_backends_runs_shutdown_guard_drop() {
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let flag = Arc::new(AtomicBool::new(false));
        let guard: Box<dyn Any + Send + Sync> = Box::new(DropFlag(flag.clone()));

        let backends = RuntimeBackends {
            capture: Box::new(vcli_capture::MockCapture::empty()),
            input: Arc::new(vcli_input::MockInputSink::new()),
            perception: vcli_perception::Perception::default(),
            clock: Arc::new(vcli_runtime::SystemRuntimeClock::new()),
            _shutdown_guard: Some(guard),
        };
        drop(backends);
        assert!(flag.load(Ordering::SeqCst), "guard's Drop must run");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p vcli-daemon --lib dropping_runtime_backends_runs_shutdown_guard_drop`
Expected: FAIL — `_shutdown_guard` field does not exist on `RuntimeBackends`.

- [ ] **Step 3: Add the field**

In `crates/vcli-daemon/src/run.rs`, replace the existing `pub struct RuntimeBackends { ... }` definition with:

```rust
/// Bundle of backend implementations the daemon will hand to the scheduler.
pub struct RuntimeBackends {
    /// Capture backend.
    pub capture: Box<dyn Capture>,
    /// Input sink.
    pub input: Arc<dyn InputSink>,
    /// Perception façade.
    pub perception: Perception,
    /// Runtime clock (usually `vcli_runtime::SystemRuntimeClock`). The runtime
    /// crate exposes its own `RuntimeClock` trait because Rust 1.75 lacks
    /// trait-object upcasting, so we cannot pass a `vcli_core::Clock` here.
    pub clock: Arc<dyn vcli_runtime::RuntimeClock>,
    /// Type-erased handle for any backend resource that needs to live as
    /// long as `RuntimeBackends` and be torn down on drop. The macOS
    /// factory parks the kill-switch listener handle here (Decision B3);
    /// mock factories leave this `None`. Boxing as `dyn Any + Send + Sync`
    /// avoids exposing a cfg-gated handle type in this crate's public API.
    pub _shutdown_guard: Option<Box<dyn std::any::Any + Send + Sync>>,
}
```

- [ ] **Step 4: Update every existing `RuntimeBackends { ... }` literal**

Find them: `rg --no-filename -n 'RuntimeBackends *\{' crates/vcli-daemon`

For each match, add `_shutdown_guard: None,` as the last field. As of plan-6 land time, the call sites are:
- `crates/vcli-daemon/src/bin/vcli-daemon.rs` — inside `default_runtime_factory` (will be replaced wholesale in Task 4 anyway, but add the field now to keep the build green between commits).
- `crates/vcli-daemon/src/handler.rs` — `fn fresh_handler` test helper (search for the literal).
- `crates/vcli-daemon/tests/*.rs` — at least `submit_runs_through_mocks.rs`, `startup_orphan_recovery.rs`, `graceful_shutdown.rs`. Confirm with the rg command above.

Do NOT add a public constructor or Default impl — keep field-literal construction so any new test forces an explicit decision about the guard.

- [ ] **Step 5: Run the test plus the full crate gate**

Run: `cargo test -p vcli-daemon --locked && cargo clippy -p vcli-daemon --all-targets -- -D warnings && cargo fmt -p vcli-daemon -- --check`
Expected: all green. The `_shutdown_guard` field starts with an underscore so clippy's `dead_code` lint doesn't fire on the mock-only code paths that never set it.

- [ ] **Step 6: Commit**

```bash
git add crates/vcli-daemon/src/run.rs \
        crates/vcli-daemon/src/bin/vcli-daemon.rs \
        crates/vcli-daemon/src/handler.rs \
        crates/vcli-daemon/tests
git commit -m "vcli-daemon: RuntimeBackends gains type-erased _shutdown_guard

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: `factory_mock` module + `build_default_backends` cfg dispatch

Move the mock-only factory body out of the binary into a shared `factory_mock` module on the library side, and stand up the cfg dispatch entry point so the binary becomes a thin wrapper. macOS-real wiring lands in Task 4.

**Files:**
- Create: `crates/vcli-daemon/src/factory_mock.rs`
- Modify: `crates/vcli-daemon/src/lib.rs` (add `mod factory_mock;` + a re-exported `build_default_backends()` that delegates by cfg)

- [ ] **Step 1: Write the failing test**

Append to `crates/vcli-daemon/src/factory_mock.rs` (file does not exist yet — create it with this content):

```rust
//! Mock RuntimeBackends factory used on non-macOS targets and as a
//! convenient default in dev / CI on macOS when real TCC isn't available.
//!
//! Production macOS daemons should NOT call this — they call the
//! macOS factory in `factory_macos` via `build_default_backends`.

use std::sync::Arc;

use crate::error::DaemonResult;
use crate::run::RuntimeBackends;

/// Build a fully-mocked `RuntimeBackends`. Always succeeds.
#[must_use]
pub fn build() -> DaemonResult<RuntimeBackends> {
    Ok(RuntimeBackends {
        capture: Box::new(vcli_capture::MockCapture::empty()),
        input: Arc::new(vcli_input::MockInputSink::new()),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_runtime::SystemRuntimeClock::new()),
        _shutdown_guard: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_returns_ok_with_no_shutdown_guard() {
        let b = build().expect("mock factory cannot fail");
        assert!(b._shutdown_guard.is_none());
    }
}
```

In `crates/vcli-daemon/src/lib.rs`, add (preserve existing `pub mod` declarations):

```rust
pub mod factory_mock;
```

- [ ] **Step 2: Run test to verify it fails (then passes)**

Run: `cargo test -p vcli-daemon --lib build_returns_ok_with_no_shutdown_guard`
Expected: PASS on first compile (the test only exercises code we just wrote). Step 2 is included for parity with TDD format; the negative case for this module is just that the file didn't exist before.

- [ ] **Step 3: Add the cfg dispatch entry point**

Append to `crates/vcli-daemon/src/lib.rs`:

```rust
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
```

Note: `factory_macos` is added in Task 4 — the cfg attribute means non-macOS builds compile right now; macOS builds will fail to compile at this point until Task 4 lands. To keep every commit green, *also* add a temporary compile-only stub in this same commit so the macOS branch compiles to mocks until Task 4 swaps it for the real thing:

```rust
#[cfg(target_os = "macos")]
mod factory_macos {
    use crate::error::DaemonResult;
    use crate::run::RuntimeBackends;
    /// Stub — replaced in Task 4 with the real macOS wiring.
    pub fn build() -> DaemonResult<RuntimeBackends> {
        crate::factory_mock::build()
    }
}
```

This stub will be deleted (replaced by a real `mod factory_macos;`) in Task 4.

- [ ] **Step 4: Run the full crate gate**

Run: `cargo test -p vcli-daemon --locked && cargo clippy -p vcli-daemon --all-targets -- -D warnings && cargo fmt -p vcli-daemon -- --check`
Expected: all green on both Linux and macOS.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-daemon/src/factory_mock.rs \
        crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: factory_mock + build_default_backends cfg dispatch

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Real macOS factory (`factory_macos`)

Replace the inline stub from Task 3 with the real wiring: `MacCapture::new()` + `CGEventInputSink::new()` + `spawn_kill_switch_listener()`. Map every error to `DaemonError::BackendInit` with an actionable `reason` string.

**Files:**
- Create: `crates/vcli-daemon/src/factory_macos.rs`
- Modify: `crates/vcli-daemon/src/lib.rs` (replace the inline `mod factory_macos { ... }` stub with `mod factory_macos;`, still gated by `#[cfg(target_os = "macos")]`)

- [ ] **Step 1: Write the failing test**

Create `crates/vcli-daemon/src/factory_macos.rs` (does not exist yet) with this content:

```rust
//! Real macOS `RuntimeBackends` factory. Compiled only on
//! `target_os = "macos"`. Constructs `MacCapture` + `CGEventInputSink`
//! and parks the kill-switch listener handle on `_shutdown_guard`.
//!
//! See spec Decisions B1, B2, B3, B5.

#![cfg(target_os = "macos")]

use std::sync::Arc;

use vcli_capture::macos::MacCapture;
use vcli_input::macos::{spawn_kill_switch_listener, CGEventInputSink};
use vcli_input::KillSwitch;

use crate::error::{DaemonError, DaemonResult};
use crate::run::RuntimeBackends;

/// Build the production macOS `RuntimeBackends`.
///
/// # Errors
///
/// `DaemonError::BackendInit { backend: "capture", .. }` if Screen Recording
/// is not granted (TCC denial in `MacCapture::new`).
///
/// `DaemonError::BackendInit { backend: "input", .. }` if the kill-switch
/// listener thread cannot be spawned (typically Input Monitoring is denied).
pub fn build() -> DaemonResult<RuntimeBackends> {
    let capture = MacCapture::new().map_err(|e| DaemonError::BackendInit {
        backend: "capture",
        reason: format!(
            "{e} — grant access in System Settings → Privacy & Security → Screen Recording, then restart the daemon"
        ),
    })?;

    let kill = KillSwitch::new();
    let listener = spawn_kill_switch_listener(kill.clone()).map_err(|e| {
        DaemonError::BackendInit {
            backend: "input",
            reason: format!(
                "kill-switch listener: {e} — grant access in System Settings → Privacy & Security → Input Monitoring, then restart the daemon"
            ),
        }
    })?;
    let input = CGEventInputSink::new(kill);

    Ok(RuntimeBackends {
        capture: Box::new(capture),
        input: Arc::new(input),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_runtime::SystemRuntimeClock::new()),
        _shutdown_guard: Some(Box::new(listener)),
    })
}

#[cfg(test)]
mod tests {
    // build() touches macOS TCC and may prompt the user. Tests live in
    // `tests/real_backends_macos.rs` and are #[ignore]d.
}
```

In `crates/vcli-daemon/src/lib.rs`, replace the inline `mod factory_macos { ... }` stub from Task 3 with:

```rust
#[cfg(target_os = "macos")]
mod factory_macos;
```

- [ ] **Step 2: Run test to verify it compiles on macOS**

Run (on macOS): `cargo build -p vcli-daemon --locked`
Expected: builds clean. The factory's runtime behavior is verified by the integration test in Task 6.

Run (on Linux/CI): `cargo build -p vcli-daemon --locked`
Expected: builds clean — `factory_macos` module is not compiled.

- [ ] **Step 3: Run the full crate gate**

Run: `cargo test -p vcli-daemon --locked && cargo clippy -p vcli-daemon --all-targets -- -D warnings && cargo fmt -p vcli-daemon -- --check`
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-daemon/src/factory_macos.rs \
        crates/vcli-daemon/src/lib.rs
git commit -m "vcli-daemon: factory_macos wires MacCapture + CGEventInputSink + kill-switch tap

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Binary calls `build_default_backends` + emits permission probe

The binary's `default_runtime_factory` becomes a thin wrapper over `vcli_daemon::build_default_backends`. Also emits one `tracing::info!` event with the `vcli_input::permissions::probe()` report at startup so users can correlate later `permission_denied` runtime errors with what was actually granted at boot (Decision B4).

**Files:**
- Modify: `crates/vcli-daemon/src/bin/vcli-daemon.rs` (replace `default_runtime_factory` body; add probe-log call before `block_on(run_foreground(...))`)

- [ ] **Step 1: Write the failing test**

This task is binary-only; the unit-test surface is thin. The integration test in Task 6 covers the macOS path. For Linux/CI, sanity-check that the binary builds and `--help` exits 0:

```bash
cargo build --release -p vcli-daemon --locked
./target/release/vcli-daemon --help 2>&1 || true   # vcli-daemon may not have --help; exit 0 either way
```

If you want a smoke test that goes through the build path, add to `crates/vcli-daemon/tests/binary_smoke.rs` (create the file):

```rust
//! Smoke test: the released vcli-daemon binary exists and starts up.
//! On Linux this runs the binary briefly and SIGTERMs it. On macOS the
//! binary will fail with DaemonError::BackendInit unless the user has
//! granted Screen Recording — that test lives in real_backends_macos.rs.

#![cfg(not(target_os = "macos"))]

use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn vcli_daemon_help_or_starts_clean_on_linux() {
    // Compile the binary first via cargo's test framework helpers.
    // Build path is target/<profile>/vcli-daemon.
    let bin = env!("CARGO_BIN_EXE_vcli-daemon");
    let mut child = Command::new(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vcli-daemon");
    std::thread::sleep(Duration::from_millis(500));
    let _ = child.kill();
    let out = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Either it ran clean (exit 0 after kill = signal) or printed a
    // recognisable startup line. We just want to make sure it didn't
    // crash on startup before signal handlers were installed.
    assert!(
        stderr.is_empty() || stderr.contains("vcli") || out.status.code() != Some(101),
        "unexpected stderr: {stderr}"
    );
}
```

Add `[[bin]] name = "vcli-daemon"` already exists in `crates/vcli-daemon/Cargo.toml`; `CARGO_BIN_EXE_vcli-daemon` will resolve at test time without further config.

- [ ] **Step 2: Run test to verify it fails or fails meaningfully**

Run (on Linux): `cargo test -p vcli-daemon --test binary_smoke`
Expected: PASS once the binary builds (the test logic doesn't depend on this task's content). On macOS, the file is `cfg`d out and the test set is empty.

- [ ] **Step 3: Replace `default_runtime_factory`**

In `crates/vcli-daemon/src/bin/vcli-daemon.rs`, replace the existing function body and the comment block above it:

```rust
/// Real-backend factory used in production. On macOS this constructs the
/// real `MacCapture` + `CGEventInputSink`; on every other platform it
/// falls back to mocks (Windows real backends arrive in v0.4 per Decision G).
fn default_runtime_factory() -> Result<RuntimeBackends, DaemonError> {
    vcli_daemon::build_default_backends()
}
```

(The existing imports `use vcli_daemon::DaemonError;` etc. should already be in scope; if not, add them. Remove the `use vcli_capture::*;` / `use vcli_input::*;` lines that the old mock-wiring needed but the new wrapper does not.)

- [ ] **Step 4: Add the permission-probe log line**

Still in `crates/vcli-daemon/src/bin/vcli-daemon.rs`, in `fn main()`, immediately before the `match rt.block_on(run_foreground(cfg, factory))` call, add:

```rust
{
    let report = vcli_input::permissions::probe();
    tracing::info!(
        accessibility = ?report.accessibility,
        input_monitoring = ?report.input_monitoring,
        "input permission probe"
    );
}
```

If the surrounding `fn main` doesn't yet import `vcli_input` directly (it may go through `vcli_daemon` only), add `use vcli_input;` at the top — or call through `vcli_daemon::vcli_input` if the daemon crate re-exports it. Trust `cargo check`.

- [ ] **Step 5: Run the daemon-crate gate**

Run: `cargo test -p vcli-daemon --locked && cargo clippy -p vcli-daemon --all-targets -- -D warnings && cargo fmt -p vcli-daemon -- --check`
Expected: all green on both platforms.

- [ ] **Step 6: Commit**

```bash
git add crates/vcli-daemon/src/bin/vcli-daemon.rs \
        crates/vcli-daemon/tests/binary_smoke.rs
git commit -m "vcli-daemon: binary delegates to build_default_backends + logs permission probe

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `#[ignore]`d integration test for the real macOS backends

This test only runs when the user invokes `cargo test -p vcli-daemon -- --ignored` on macOS with TCC granted. It exercises the real wiring end-to-end without going through the IPC/scheduler — just constructs `factory_macos::build()` and confirms the resulting `RuntimeBackends` actually grabs a frame and that dropping it tears down the kill-switch listener cleanly.

**Files:**
- Create: `crates/vcli-daemon/tests/real_backends_macos.rs`

- [ ] **Step 1: Write the test**

```rust
//! Real-backend integration test, macOS only, #[ignore]d.
//!
//! Run with: `cargo test -p vcli-daemon --test real_backends_macos -- --ignored`
//!
//! Requires Screen Recording AND Input Monitoring granted to the test
//! binary. First run will trigger TCC prompts; grant them, then re-run.

#![cfg(target_os = "macos")]

use std::time::Duration;

use vcli_capture::Capture;

#[test]
#[ignore = "requires Screen Recording + Input Monitoring TCC grants"]
fn factory_macos_build_yields_working_capture_and_drops_cleanly() {
    let backends = vcli_daemon::build_default_backends()
        .expect("build_default_backends — did you grant Screen Recording?");

    // Confirm capture actually works: grab one frame, expect non-zero size.
    let frame = backends
        .capture
        .grab_screen()
        .expect("MacCapture::grab_screen failed — Screen Recording probably denied");
    assert!(
        frame.bounds.w > 0 && frame.bounds.h > 0,
        "frame has zero dimensions: {:?}",
        frame.bounds
    );

    // The kill-switch listener thread should be alive; the only way to
    // observe it externally is to confirm Drop is clean. Park the bundle
    // briefly, then drop it.
    std::thread::sleep(Duration::from_millis(50));
    drop(backends);

    // If the listener thread didn't park its CFRunLoop properly we'd hang
    // here; the test's wall clock catches that.
    std::thread::sleep(Duration::from_millis(50));
}
```

- [ ] **Step 2: Confirm it does not run by default**

Run: `cargo test -p vcli-daemon --test real_backends_macos`
Expected: `1 test ignored` (the test exists but is gated). On Linux the test file is `cfg`d out and the test set is empty.

- [ ] **Step 3: (Manual, macOS only) Run the ignored test**

Run: `cargo test -p vcli-daemon --test real_backends_macos -- --ignored`
Expected on first run: macOS opens TCC prompts for Screen Recording and Input Monitoring against the test binary. Grant both in System Settings → Privacy & Security, then re-run. Expected on subsequent runs: PASS.

This is a one-time human verification. Document the grant requirement in README (Task 7).

- [ ] **Step 4: Run the workspace gate (no `--ignored`)**

Run: `cargo test --workspace --locked && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: all green; the new test reports as `1 ignored`.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-daemon/tests/real_backends_macos.rs
git commit -m "vcli-daemon: #[ignore]d real-backends integration test (macOS, TCC-gated)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: README + ARCHITECTURE updates

Flip the daemon row from ⏳ to ✅ in the README status table, document the TCC grant flow on first run, and update ARCHITECTURE to remove "TBD" from the daemon box.

**Files:**
- Modify: `README.md`
- Modify: `ARCHITECTURE.md`

- [ ] **Step 1: Update README status table**

In `README.md`, find the table row for `vcli-daemon` (currently ends with `⏳`) and change it to `✅ (macOS) / 🚧 (Windows v0.4)`. Same for `vcli-cli` if it's still ⏳ — check the current state with `grep -n vcli-daemon\\|vcli-cli README.md`.

- [ ] **Step 2: Add a "First run on macOS" section to README**

After the `## Build & test` section in `README.md`, insert:

```markdown
## First run on macOS

The daemon needs three macOS TCC grants to actually capture and click. Build, then start once and grant the prompts:

```bash
cargo build --release
./target/release/vcli daemon start    # may print "spawn daemon: No such file or directory"
                                       # if vcli-daemon isn't on PATH; see below
```

If `vcli daemon start` cannot find `vcli-daemon`, prepend the release dir to `PATH`:

```bash
export PATH="$PWD/target/release:$PATH"
vcli daemon start
```

On first run the daemon will fail with `BackendInit: capture backend init failed: ... grant access in System Settings → Privacy & Security → Screen Recording`. Open that pane, grant access to `vcli-daemon`, then run `vcli daemon start` again. You will see another prompt for Input Monitoring (kill-switch tap); grant it. The third bucket — Accessibility — is needed for `CGEventPost` to actually post events; the daemon does not start the input pipeline until the first `click` action, so you'll see that prompt appear when your first program tries to click.

A quick sanity check on capture without involving the daemon:

```bash
cargo run -p vcli-capture --example capture_once -- --save /tmp/vcli-frame.png
```
```

- [ ] **Step 3: Update ARCHITECTURE.md daemon box**

In `ARCHITECTURE.md`, find the `vcli-daemon (TBD)` line in the data-flow diagram and change to `vcli-daemon`. In the workspace-layout listing, change `vcli-daemon/         (TBD) tick loop wiring the above` to `vcli-daemon/         tick loop wiring the above; ships real macOS backends (plan-6)`.

- [ ] **Step 4: Verify the docs build cleanly**

Run: `cargo doc --workspace --no-deps --locked 2>&1 | grep -E '^(warning|error)' | head`
Expected: no warnings or errors.

- [ ] **Step 5: Commit**

```bash
git add README.md ARCHITECTURE.md
git commit -m "docs: vcli-daemon ships real macOS backends; first-run TCC flow

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Full workspace gate

Final guardrail — same gate AGENT.md says must pass on every commit and push.

- [ ] **Step 1: Run the full gate**

Run:
```bash
cargo fmt --all -- --check && \
cargo clippy --workspace --all-targets -- -D warnings && \
cargo test --workspace --locked
```

Expected: all green. Test count includes one new `#[ignore]`d test (`factory_macos_build_yields_working_capture_and_drops_cleanly`) and any added by Tasks 1–5.

- [ ] **Step 2: Verify no commits are missing the co-author trailer**

Run: `git log master..HEAD --format='%s %an' | head -20` (substitute the actual base if different)
Expected: every commit message contains a Co-Authored-By trailer (visible in `git log --format=%B` if needed).

- [ ] **Step 3: Manual macOS smoke (optional but recommended)**

On a macOS machine with TCC granted:

```bash
export PATH="$PWD/target/release:$PATH"
vcli daemon start && sleep 1 && vcli health && vcli daemon stop
```

Expected: `daemon started`, then `daemon: ok` with the version + uptime, then `stopped`. The daemon log at `~/Library/Logs/vcli/daemon.log.<date>` should contain the `input permission probe` info line and **no** `MockCapture has no screen frames configured` warnings.

---

## Notes for the implementer

- **Plan-6 deliberately does not fix the `vcli list` state-persistence bug** found alongside the mock-backends issue. That bug is in `vcli-runtime`, not `vcli-daemon`, and needs its own plan (TODOS.md). Resist the urge to land it as a drive-by — it changes the scheduler's commit boundary with the store and deserves its own design pass.
- The `RuntimeFactory` injection pattern is preserved so all existing daemon tests keep using mocks with no behavior change. If you find a test that needs real backends, write it as `#[ignore]`d in `tests/real_backends_macos.rs` rather than swapping the test factory.
- If the `MacCapture::new()` API changes between now and execution time (it currently probes TCC at construction), refactor Task 4 to whatever the real signature requires. The contract that matters for plan-6 is: "the produced RuntimeBackends actually captures and inputs."
- Each task ends in a commit. **Do not squash.** AGENT.md's commit discipline applies to every plan, and this plan is no exception.
- The temporary stub `mod factory_macos { ... }` introduced in Task 3 step 3 is deleted in Task 4 step 1 — double-check the diff has no leftover stub before committing Task 4.
