# vcli-input Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose an `InputSink` trait with macOS CGEvent implementation, `MockInputSink`, a global `KillSwitch`, and Windows stub.

**Architecture:** `InputSink` is a synchronous trait taking `&self` (interior mutability inside impls) returning `Result<(), InputError>` — every method blocks until the OS has posted/dispatched the event, giving the runtime a clean confirm-before-advance contract (spec §Action confirmation). The macOS backend is built on the `core-graphics` crate (`CGEvent`, `CGEventSource`, `CGEventTapLocation::HIDEventTap`, `CGEventTapCreate`) — chosen over `enigo` because v0 needs exact control over `CGEventField::{MouseEventClickState, KeyboardEventAutorepeat, ScrollWheelEventDeltaAxis1}` for reliable double-clicks, modifier flag masks, and scroll pixel-vs-line semantics that `enigo` papers over. Every macOS method first checks a process-global `KillSwitch` (an `Arc<AtomicBool>` held inside the sink); an always-on background `CFRunLoop` thread installs a `CGEventTap` in `kCGHeadInsertEventTap` / `kCGEventTapOptionListenOnly` mode and flips the flag on the `Cmd+Shift+Esc` chord so a panicking human can halt the runtime without needing IPC.

**Tech Stack:** Rust (stable, 2021 edition). Dependencies: `vcli-core` (workspace), `thiserror` (workspace), `core-graphics = "0.24"`, `core-foundation = "0.10"`, `libc = "0.2"` (TCC query helpers). Dev-deps: `tempfile = "3"`, `proptest = "1"` (workspace). All macOS code gated behind `#[cfg(target_os = "macos")]`; Windows code behind `#[cfg(target_os = "windows")]` with `unimplemented!()` bodies. Depends on `vcli-core::{InputAction, Button, Modifier, geom::Point}`.

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md`. When this plan references a decision by number (e.g. "Decision 1.2"), see the "Review decisions — 2026-04-16" appendix in that file. Codex Decision B (daemon architecture / HIL kill switch) is the direct driver of the `KillSwitch` design below.

---

## File structure produced by this plan

```
vcli/
├── Cargo.toml                                 # MODIFY: add vcli-input member + workspace deps
└── crates/
    └── vcli-input/
        ├── Cargo.toml
        ├── README.md
        ├── src/
        │   ├── lib.rs                         # module tree + re-exports + doctest
        │   ├── error.rs                       # InputError enum (thiserror)
        │   ├── sink.rs                        # InputSink trait + DragSegment helper
        │   ├── mock.rs                        # MockInputSink (records InputAction log)
        │   ├── kill_switch.rs                 # KillSwitch primitive (Arc<AtomicBool> + observer)
        │   ├── permissions.rs                 # PermissionStatus + diagnostics (TCC query)
        │   ├── keymap.rs                      # canonical key-name -> CGKeyCode + virtual-key table
        │   ├── macos/
        │   │   ├── mod.rs                     # cfg(target_os = "macos") entry point
        │   │   ├── cg_sink.rs                 # CGEventInputSink (real backend)
        │   │   ├── cg_events.rs               # low-level CGEvent helpers (move/click/scroll/key)
        │   │   ├── cg_typing.rs               # type_text → Unicode string events
        │   │   ├── hotkey_tap.rs              # CGEventTap listener thread for kill-switch chord
        │   │   └── tcc.rs                     # Accessibility + Input Monitoring probes
        │   └── windows/
        │       └── mod.rs                     # cfg(target_os = "windows") stub impl
        └── tests/
            ├── mock_contract.rs               # InputSink contract verified against MockInputSink
            ├── kill_switch.rs                 # KillSwitch semantics (halts mock mid-sequence)
            └── macos_real.rs                  # #[ignore] real CGEvent smoke tests
```

**Responsibility split rationale:** the trait, error, kill switch, and mock live in platform-agnostic files at the crate root so any downstream consumer gets them on every OS. The `macos/` and `windows/` submodules are each hidden behind a single `#[cfg(...)]` so the crate still compiles on Linux CI runners (macOS-only types vanish; Windows stubs don't exist). `keymap.rs` is cross-platform data because both backends need to translate the canonical vcli key name to a platform virtual key. No file exceeds ~350 lines.

---

### Task 1: Register `vcli-input` as a workspace member

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Verify current workspace state**

Run: `cargo check --workspace`
Expected: builds OK with only `vcli-core`.

- [ ] **Step 2: Add member + workspace deps**

In `Cargo.toml` under `[workspace]`, change `members` to:

```toml
[workspace]
resolver = "2"
members = [
    "crates/vcli-core",
    "crates/vcli-input",
]
```

In `[workspace.dependencies]`, append:

```toml
# Platform-specific (used by vcli-input + vcli-capture later)
core-graphics = "0.24"
core-foundation = "0.10"
libc = "0.2"
tempfile = "3"
```

- [ ] **Step 3: Verify parse (will fail until crate exists)**

Run: `cargo check --workspace`
Expected: error `failed to load manifest for workspace member crates/vcli-input`. Accept; Task 2 creates the crate.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "vcli-input: register workspace member + add core-graphics deps"
```

---

### Task 2: Scaffold empty `vcli-input` crate

**Files:**
- Create: `crates/vcli-input/Cargo.toml`
- Create: `crates/vcli-input/src/lib.rs`
- Create: `crates/vcli-input/README.md`

- [ ] **Step 1: Write `crates/vcli-input/Cargo.toml`**

```toml
[package]
name = "vcli-input"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Input synthesis trait, macOS CGEvent backend, mock, and HIL kill switch for vcli."

[dependencies]
vcli-core = { path = "../vcli-core" }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }

[target.'cfg(target_os = "macos")'.dependencies]
core-graphics = { workspace = true }
core-foundation = { workspace = true }
libc = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
proptest = { workspace = true }
```

- [ ] **Step 2: Write minimal `src/lib.rs`**

```rust
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
```

- [ ] **Step 3: Write `README.md`**

```markdown
# vcli-input

Input synthesis. Real macOS CGEvent backend, a mock, and a human-in-the-loop kill switch.

See the design spec at `docs/superpowers/specs/2026-04-16-vcli-design.md` §Input synthesis.
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p vcli-input`
Expected: builds clean.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-input/Cargo.toml crates/vcli-input/README.md crates/vcli-input/src/lib.rs
git commit -m "vcli-input: empty crate shell with README"
```

---

### Task 3: `InputError` enum

**Files:**
- Create: `crates/vcli-input/src/error.rs`
- Modify: `crates/vcli-input/src/lib.rs`

- [ ] **Step 1: Add `mod error;` + re-export**

Append to `src/lib.rs`:

```rust
pub mod error;

pub use error::InputError;
```

- [ ] **Step 2: Write failing tests in `src/error.rs`**

```rust
//! Errors surfaced by [`InputSink`] implementations.

use thiserror::Error;
use vcli_core::ErrorCode;

/// Error returned by every `InputSink` method. `Halted` wins over all others
/// because the kill switch short-circuits before the backend is even called.
#[derive(Debug, Error)]
pub enum InputError {
    /// The kill switch was set; no OS event was posted.
    #[error("input halted: kill switch is engaged")]
    Halted,

    /// macOS TCC (Accessibility / Input Monitoring) has not granted this process
    /// permission to synthesize input.
    #[error("input permission denied: {detail}")]
    PermissionDenied {
        /// Short human-readable reason — which TCC bucket is missing.
        detail: String,
    },

    /// Event creation / posting failed at the OS layer.
    #[error("backend failure: {detail}")]
    Backend {
        /// Error message from the OS / FFI call.
        detail: String,
    },

    /// Unknown key name in a key-combo action.
    #[error("unknown key: {0}")]
    UnknownKey(String),

    /// Invalid argument (e.g. empty drag path, `hold_ms > 60_000`).
    #[error("invalid input argument: {0}")]
    InvalidArgument(String),

    /// Platform does not support this method (Windows stub in v0).
    #[error("not implemented on this platform")]
    Unimplemented,
}

impl InputError {
    /// Map to the IPC wire-level [`ErrorCode`]. `Halted` maps to `internal`
    /// because it is a local-only condition with no external counterpart.
    #[must_use]
    pub fn to_error_code(&self) -> ErrorCode {
        match self {
            Self::Halted | Self::Backend { .. } | Self::Unimplemented => ErrorCode::Internal,
            Self::PermissionDenied { .. } => ErrorCode::PermissionDenied,
            Self::UnknownKey(_) | Self::InvalidArgument(_) => ErrorCode::InvalidProgram,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halted_display_is_stable() {
        assert_eq!(
            InputError::Halted.to_string(),
            "input halted: kill switch is engaged"
        );
    }

    #[test]
    fn permission_maps_to_permission_denied() {
        let e = InputError::PermissionDenied {
            detail: "Accessibility".into(),
        };
        assert_eq!(e.to_error_code(), ErrorCode::PermissionDenied);
    }

    #[test]
    fn unknown_key_maps_to_invalid_program() {
        let e = InputError::UnknownKey("blargh".into());
        assert_eq!(e.to_error_code(), ErrorCode::InvalidProgram);
        assert!(e.to_string().contains("blargh"));
    }

    #[test]
    fn halted_maps_to_internal() {
        assert_eq!(InputError::Halted.to_error_code(), ErrorCode::Internal);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-input --lib error`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-input/src/error.rs crates/vcli-input/src/lib.rs
git commit -m "vcli-input: InputError (thiserror) + ErrorCode mapping"
```

---

### Task 4: `KillSwitch` primitive

**Files:**
- Create: `crates/vcli-input/src/kill_switch.rs`
- Modify: `crates/vcli-input/src/lib.rs`

- [ ] **Step 1: Add `mod kill_switch;` + re-export**

Append to `src/lib.rs`:

```rust
pub mod kill_switch;

pub use kill_switch::{KillSwitch, KillSwitchObserver};
```

- [ ] **Step 2: Write `kill_switch.rs` tests first**

```rust
//! Process-global HIL "stop everything" flag (Codex Decision B).
//!
//! Cheap to clone (internal `Arc<AtomicBool>`). Every `InputSink` method must
//! check `is_engaged()` before calling into the OS and return `InputError::Halted`
//! if set. The flag is observable (`subscribe`) so higher-level runtimes can
//! cancel waits as soon as a human panics.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Thread-safe kill switch. Cloning shares the underlying flag.
#[derive(Debug, Clone, Default)]
pub struct KillSwitch {
    engaged: Arc<AtomicBool>,
    engaged_at_ns: Arc<AtomicU64>,
}

use std::sync::atomic::AtomicU64;

impl KillSwitch {
    /// Fresh switch, not engaged.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the switch. Idempotent. Stamps the engagement time (monotonic ns since
    /// an unspecified epoch) for observability.
    pub fn engage(&self) {
        let now_ns = duration_since_boot_ns();
        // CAS so we don't overwrite the original engagement timestamp on re-engage.
        let _ = self.engaged_at_ns.compare_exchange(
            0,
            now_ns,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        self.engaged.store(true, Ordering::SeqCst);
    }

    /// Clear the switch. Used only by tests and admin paths; production never clears.
    pub fn disengage(&self) {
        self.engaged.store(false, Ordering::SeqCst);
        self.engaged_at_ns.store(0, Ordering::SeqCst);
    }

    /// Non-blocking check.
    #[must_use]
    pub fn is_engaged(&self) -> bool {
        self.engaged.load(Ordering::SeqCst)
    }

    /// Observer handle for other crates (e.g. `vcli-runtime`) that want to wake
    /// their waits when the switch flips. Busy-wait implementation is acceptable
    /// because engagement is a rare, terminal event.
    #[must_use]
    pub fn subscribe(&self) -> KillSwitchObserver {
        KillSwitchObserver {
            switch: self.clone(),
        }
    }
}

/// Observer returned by [`KillSwitch::subscribe`].
#[derive(Debug, Clone)]
pub struct KillSwitchObserver {
    switch: KillSwitch,
}

impl KillSwitchObserver {
    /// Return immediately if engaged; otherwise poll every `poll` until
    /// `timeout` elapses. Returns whether the switch became engaged.
    pub fn wait_until_engaged(&self, timeout: Duration, poll: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.switch.is_engaged() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(poll.min(Duration::from_millis(50)));
        }
    }
}

fn duration_since_boot_ns() -> u64 {
    // Monotonic-ish "now" — does not need to be synced across machines.
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos() & u128::from(u64::MAX)).unwrap_or(0))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn fresh_switch_is_not_engaged() {
        let k = KillSwitch::new();
        assert!(!k.is_engaged());
    }

    #[test]
    fn engage_is_observable_from_clone() {
        let k = KillSwitch::new();
        let clone = k.clone();
        assert!(!clone.is_engaged());
        k.engage();
        assert!(clone.is_engaged());
    }

    #[test]
    fn engage_is_idempotent() {
        let k = KillSwitch::new();
        k.engage();
        let first = k.engaged_at_ns.load(Ordering::SeqCst);
        k.engage();
        let second = k.engaged_at_ns.load(Ordering::SeqCst);
        assert_eq!(first, second, "timestamp must not change on re-engage");
    }

    #[test]
    fn disengage_resets_flag() {
        let k = KillSwitch::new();
        k.engage();
        k.disengage();
        assert!(!k.is_engaged());
    }

    #[test]
    fn observer_returns_true_on_engage_before_timeout() {
        let k = KillSwitch::new();
        let obs = k.subscribe();
        let k2 = k.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            k2.engage();
        });
        assert!(obs.wait_until_engaged(Duration::from_millis(500), Duration::from_millis(5)));
        handle.join().unwrap();
    }

    #[test]
    fn observer_returns_false_on_timeout() {
        let k = KillSwitch::new();
        let obs = k.subscribe();
        assert!(!obs.wait_until_engaged(Duration::from_millis(20), Duration::from_millis(5)));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-input --lib kill_switch`
Expected: 6 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-input/src/kill_switch.rs crates/vcli-input/src/lib.rs
git commit -m "vcli-input: KillSwitch primitive (Arc<AtomicBool> + observer)"
```

---

### Task 5: `InputSink` trait + `DragSegment`

**Files:**
- Create: `crates/vcli-input/src/sink.rs`
- Modify: `crates/vcli-input/src/lib.rs`

- [ ] **Step 1: Add `mod sink;` + re-export**

Append to `src/lib.rs`:

```rust
pub mod sink;

pub use sink::{DragSegment, InputSink};
```

- [ ] **Step 2: Write `sink.rs` with trait and smoke tests against a dummy impl**

```rust
//! `InputSink` — the synchronous, OS-confirmed input interface.
//!
//! Every method must return only after the OS has dispatched the event
//! (microseconds, not visual confirmation — the runtime layers postcondition
//! checks on top per spec §Action confirmation). `Result<(), InputError>`
//! means "the OS accepted the event" or "we failed before posting". A
//! `KillSwitch` engaged at entry produces `InputError::Halted` before any
//! OS call.

use std::time::Duration;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use crate::error::InputError;

/// One hop of a multi-point drag. The first segment begins at `from`; later
/// segments interpolate from the prior segment's `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragSegment {
    /// End point of this segment in logical pixels.
    pub to: Point,
    /// Duration of this segment. Backends emit interpolated move events over
    /// this span to mimic a human-speed drag.
    pub duration: Duration,
}

/// Synchronous input trait. Implementors must be `Send + Sync` so the runtime
/// can hold one behind an `Arc`.
pub trait InputSink: Send + Sync {
    /// Move the cursor to `to` immediately (no interpolation).
    fn mouse_move(&self, to: Point) -> Result<(), InputError>;

    /// Click `button` at `at` with `modifiers` held. `hold_ms` is the down→up
    /// gap (0 = fire down+up back-to-back). Backends that cannot honor hold
    /// duration still must return `Ok(())` only after the up event posts.
    fn click(
        &self,
        at: Point,
        button: Button,
        modifiers: &[Modifier],
        hold_ms: u32,
    ) -> Result<(), InputError>;

    /// Double-click `button` at `at`. Implementations set the
    /// `MouseEventClickState` field to 2 on the second press so the OS treats
    /// it as a double-click (not two single clicks).
    fn double_click(&self, at: Point, button: Button) -> Result<(), InputError>;

    /// Drag from `from` through each `DragSegment.to` in order. `button` is
    /// held down across all segments and released at the final `to`. Returns
    /// only after the final mouse-up is posted.
    fn drag(
        &self,
        from: Point,
        segments: &[DragSegment],
        button: Button,
    ) -> Result<(), InputError>;

    /// Type literal UTF-8 text. Backends use Unicode key events (macOS:
    /// `CGEventKeyboardSetUnicodeString`) so the active keyboard layout is
    /// respected and arbitrary code-points (including non-ASCII) type correctly.
    fn type_text(&self, text: &str) -> Result<(), InputError>;

    /// Press a key combo. `key` uses the vcli canonical key-name set
    /// (e.g. `"s"`, `"return"`, `"space"`, `"f1"`); see [`crate::keymap`].
    /// Down/up pairs are emitted for each modifier around the primary key.
    fn key_combo(&self, modifiers: &[Modifier], key: &str) -> Result<(), InputError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal stand-in impl to prove the trait is object-safe and compiles.
    struct Nop;
    impl InputSink for Nop {
        fn mouse_move(&self, _: Point) -> Result<(), InputError> { Ok(()) }
        fn click(&self, _: Point, _: Button, _: &[Modifier], _: u32) -> Result<(), InputError> { Ok(()) }
        fn double_click(&self, _: Point, _: Button) -> Result<(), InputError> { Ok(()) }
        fn drag(&self, _: Point, _: &[DragSegment], _: Button) -> Result<(), InputError> { Ok(()) }
        fn type_text(&self, _: &str) -> Result<(), InputError> { Ok(()) }
        fn key_combo(&self, _: &[Modifier], _: &str) -> Result<(), InputError> { Ok(()) }
    }

    #[test]
    fn trait_is_object_safe() {
        let s: Box<dyn InputSink> = Box::new(Nop);
        s.mouse_move(Point { x: 1, y: 2 }).unwrap();
    }

    #[test]
    fn drag_segment_is_copy() {
        let s = DragSegment { to: Point { x: 5, y: 5 }, duration: Duration::from_millis(100) };
        let s2 = s;
        assert_eq!(s.to, s2.to);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-input --lib sink`
Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-input/src/sink.rs crates/vcli-input/src/lib.rs
git commit -m "vcli-input: InputSink trait + DragSegment"
```

---

### Task 6: `MockInputSink` — records `InputAction` log

**Files:**
- Create: `crates/vcli-input/src/mock.rs`
- Modify: `crates/vcli-input/src/lib.rs`

- [ ] **Step 1: Add `mod mock;` + re-export**

Append to `src/lib.rs`:

```rust
pub mod mock;

pub use mock::MockInputSink;
```

- [ ] **Step 2: Write `mock.rs` with tests first**

```rust
//! `MockInputSink` — records every call as an [`InputAction`] for assertions
//! in downstream crates' scenario tests. Also honors the [`KillSwitch`] so
//! kill-switch semantics are testable without an OS backend.

use std::sync::Mutex;
use std::time::Duration;

use vcli_core::action::{Button, InputAction, Modifier};
use vcli_core::geom::Point;

use crate::error::InputError;
use crate::kill_switch::KillSwitch;
use crate::sink::{DragSegment, InputSink};

/// An entry in the mock call log. Mostly 1-to-1 with [`InputAction`], but adds
/// variants the DSL-level action enum doesn't carry (`DoubleClick`, `Drag`).
#[derive(Debug, Clone, PartialEq)]
pub enum MockCall {
    /// Low-level action the runtime would emit.
    Action(InputAction),
    /// Double-click at a point with a button.
    DoubleClick {
        /// Point.
        at: Point,
        /// Button.
        button: Button,
    },
    /// Drag from `from` through segment endpoints with `button` held.
    Drag {
        /// Start point.
        from: Point,
        /// Endpoints (duration omitted — mock does not sleep).
        to: Vec<Point>,
        /// Held button.
        button: Button,
    },
    /// A click variant that carries modifiers + hold (InputAction::Click drops them).
    ClickDetailed {
        /// Point.
        at: Point,
        /// Button.
        button: Button,
        /// Modifiers.
        modifiers: Vec<Modifier>,
        /// Hold-down time in ms.
        hold_ms: u32,
    },
}

/// Recording `InputSink`. Thread-safe.
#[derive(Debug, Default)]
pub struct MockInputSink {
    log: Mutex<Vec<MockCall>>,
    kill: KillSwitch,
    /// Optional artificial error; when set, all methods return this. Useful to
    /// exercise the runtime's error-handling paths.
    forced_error: Mutex<Option<String>>,
}

impl MockInputSink {
    /// Fresh mock with an unengaged kill switch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build with a caller-provided kill switch so tests can trigger `Halted`.
    #[must_use]
    pub fn with_kill_switch(kill: KillSwitch) -> Self {
        Self { log: Mutex::new(Vec::new()), kill, forced_error: Mutex::new(None) }
    }

    /// Read the current call log.
    #[must_use]
    pub fn calls(&self) -> Vec<MockCall> {
        self.log.lock().unwrap().clone()
    }

    /// Drain and return calls (clears the log).
    pub fn drain(&self) -> Vec<MockCall> {
        std::mem::take(&mut *self.log.lock().unwrap())
    }

    /// Reference to the underlying kill switch (clone to share).
    #[must_use]
    pub fn kill_switch(&self) -> KillSwitch {
        self.kill.clone()
    }

    /// Force every subsequent call to fail with `InputError::Backend { detail }`.
    pub fn fail_with(&self, detail: impl Into<String>) {
        *self.forced_error.lock().unwrap() = Some(detail.into());
    }

    fn check(&self) -> Result<(), InputError> {
        if self.kill.is_engaged() {
            return Err(InputError::Halted);
        }
        if let Some(d) = self.forced_error.lock().unwrap().clone() {
            return Err(InputError::Backend { detail: d });
        }
        Ok(())
    }

    fn push(&self, c: MockCall) {
        self.log.lock().unwrap().push(c);
    }
}

impl InputSink for MockInputSink {
    fn mouse_move(&self, to: Point) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::Action(InputAction::Move { at: to }));
        Ok(())
    }

    fn click(
        &self,
        at: Point,
        button: Button,
        modifiers: &[Modifier],
        hold_ms: u32,
    ) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::ClickDetailed {
            at,
            button,
            modifiers: modifiers.to_vec(),
            hold_ms,
        });
        Ok(())
    }

    fn double_click(&self, at: Point, button: Button) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::DoubleClick { at, button });
        Ok(())
    }

    fn drag(
        &self,
        from: Point,
        segments: &[DragSegment],
        button: Button,
    ) -> Result<(), InputError> {
        self.check()?;
        if segments.is_empty() {
            return Err(InputError::InvalidArgument("drag segments must be non-empty".into()));
        }
        self.push(MockCall::Drag {
            from,
            to: segments.iter().map(|s| s.to).collect(),
            button,
        });
        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::Action(InputAction::Type { text: text.to_owned() }));
        Ok(())
    }

    fn key_combo(&self, modifiers: &[Modifier], key: &str) -> Result<(), InputError> {
        self.check()?;
        self.push(MockCall::Action(InputAction::Key {
            key: key.to_owned(),
            modifiers: modifiers.to_vec(),
        }));
        Ok(())
    }
}

// Silence unused-import warning when Duration isn't used after refactor.
#[allow(dead_code)]
fn _duration_in_scope(_: Duration) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_records_move() {
        let m = MockInputSink::new();
        m.mouse_move(Point { x: 3, y: 4 }).unwrap();
        assert_eq!(
            m.calls(),
            vec![MockCall::Action(InputAction::Move { at: Point { x: 3, y: 4 } })]
        );
    }

    #[test]
    fn mock_records_click_with_modifiers() {
        let m = MockInputSink::new();
        m.click(Point { x: 1, y: 1 }, Button::Left, &[Modifier::Cmd], 120).unwrap();
        assert_eq!(
            m.calls(),
            vec![MockCall::ClickDetailed {
                at: Point { x: 1, y: 1 },
                button: Button::Left,
                modifiers: vec![Modifier::Cmd],
                hold_ms: 120,
            }]
        );
    }

    #[test]
    fn mock_records_double_click_and_drag() {
        let m = MockInputSink::new();
        m.double_click(Point { x: 0, y: 0 }, Button::Left).unwrap();
        m.drag(
            Point { x: 0, y: 0 },
            &[DragSegment { to: Point { x: 10, y: 10 }, duration: Duration::from_millis(50) }],
            Button::Left,
        )
        .unwrap();
        let calls = m.calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], MockCall::DoubleClick { .. }));
        assert!(matches!(calls[1], MockCall::Drag { .. }));
    }

    #[test]
    fn mock_records_type_and_key() {
        let m = MockInputSink::new();
        m.type_text("hi").unwrap();
        m.key_combo(&[Modifier::Cmd, Modifier::Shift], "s").unwrap();
        assert_eq!(
            m.calls(),
            vec![
                MockCall::Action(InputAction::Type { text: "hi".into() }),
                MockCall::Action(InputAction::Key {
                    key: "s".into(),
                    modifiers: vec![Modifier::Cmd, Modifier::Shift],
                }),
            ]
        );
    }

    #[test]
    fn mock_rejects_empty_drag() {
        let m = MockInputSink::new();
        let e = m.drag(Point { x: 0, y: 0 }, &[], Button::Left).unwrap_err();
        matches!(e, InputError::InvalidArgument(_));
    }

    #[test]
    fn drain_empties_the_log() {
        let m = MockInputSink::new();
        m.mouse_move(Point { x: 0, y: 0 }).unwrap();
        let first = m.drain();
        assert_eq!(first.len(), 1);
        assert!(m.calls().is_empty());
    }

    #[test]
    fn forced_error_produces_backend_failure() {
        let m = MockInputSink::new();
        m.fail_with("flaky");
        let e = m.mouse_move(Point { x: 0, y: 0 }).unwrap_err();
        assert!(matches!(e, InputError::Backend { .. }));
        assert!(m.calls().is_empty(), "failed call must not be recorded");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-input --lib mock`
Expected: 7 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-input/src/mock.rs crates/vcli-input/src/lib.rs
git commit -m "vcli-input: MockInputSink with recording log + forced-error hook"
```

---

### Task 7: Integration test — kill switch halts mock mid-sequence

**Files:**
- Create: `crates/vcli-input/tests/kill_switch.rs`

- [ ] **Step 1: Write `tests/kill_switch.rs`**

```rust
//! End-to-end behavior: engaging the kill switch makes every subsequent call
//! fail with `Halted` and stops recording. Previously recorded calls stay.

use std::time::Duration;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use vcli_input::kill_switch::KillSwitch;
use vcli_input::mock::{MockCall, MockInputSink};
use vcli_input::sink::{DragSegment, InputSink};
use vcli_input::InputError;

#[test]
fn engage_halts_every_subsequent_method() {
    let kill = KillSwitch::new();
    let mock = MockInputSink::with_kill_switch(kill.clone());

    // Before engage: all methods record.
    mock.mouse_move(Point { x: 1, y: 1 }).unwrap();
    mock.click(Point { x: 1, y: 1 }, Button::Left, &[], 0).unwrap();
    mock.double_click(Point { x: 2, y: 2 }, Button::Left).unwrap();
    mock.drag(
        Point { x: 0, y: 0 },
        &[DragSegment { to: Point { x: 5, y: 5 }, duration: Duration::from_millis(5) }],
        Button::Left,
    )
    .unwrap();
    mock.type_text("hello").unwrap();
    mock.key_combo(&[Modifier::Cmd], "s").unwrap();
    assert_eq!(mock.calls().len(), 6);

    // Engage.
    kill.engage();

    for result in [
        mock.mouse_move(Point { x: 0, y: 0 }),
        mock.click(Point { x: 0, y: 0 }, Button::Left, &[], 0),
        mock.double_click(Point { x: 0, y: 0 }, Button::Left),
        mock.drag(
            Point { x: 0, y: 0 },
            &[DragSegment { to: Point { x: 1, y: 1 }, duration: Duration::from_millis(1) }],
            Button::Left,
        ),
        mock.type_text("nope"),
        mock.key_combo(&[], "a"),
    ] {
        let e = result.unwrap_err();
        assert!(matches!(e, InputError::Halted), "got {e:?}");
    }

    // Recorded calls are unchanged (no new ones added after engage).
    assert_eq!(mock.calls().len(), 6);
}

#[test]
fn observer_wakes_immediately_when_switch_engaged_concurrently() {
    let kill = KillSwitch::new();
    let obs = kill.subscribe();
    let k2 = kill.clone();
    let h = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        k2.engage();
    });
    assert!(obs.wait_until_engaged(Duration::from_secs(1), Duration::from_millis(2)));
    h.join().unwrap();
}

#[test]
fn disengage_reenables_calls() {
    let kill = KillSwitch::new();
    let mock = MockInputSink::with_kill_switch(kill.clone());
    kill.engage();
    assert!(matches!(
        mock.mouse_move(Point { x: 1, y: 1 }).unwrap_err(),
        InputError::Halted
    ));
    kill.disengage();
    mock.mouse_move(Point { x: 2, y: 2 }).unwrap();
    assert_eq!(
        mock.calls(),
        vec![MockCall::Action(vcli_core::action::InputAction::Move {
            at: Point { x: 2, y: 2 }
        })]
    );
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p vcli-input --test kill_switch`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-input/tests/kill_switch.rs
git commit -m "vcli-input: integration test — kill switch halts mock mid-sequence"
```

---

### Task 8: Keymap (canonical key name → platform virtual keycode)

**Files:**
- Create: `crates/vcli-input/src/keymap.rs`
- Modify: `crates/vcli-input/src/lib.rs`

- [ ] **Step 1: Add `mod keymap;` + re-export**

Append to `src/lib.rs`:

```rust
pub mod keymap;

pub use keymap::{CanonicalKey, macos_keycode};
```

- [ ] **Step 2: Write `keymap.rs` with tests first**

```rust
//! Mapping from the vcli canonical key-name vocabulary to platform virtual
//! keycodes. Kept cross-platform so the Windows backend has the same parsing.
//!
//! Canonical names are lowercase ASCII, e.g. `"a"`, `"return"`, `"space"`,
//! `"tab"`, `"escape"`, `"left"`, `"right"`, `"up"`, `"down"`, `"f1"`..`"f12"`,
//! `"backspace"`, `"delete"`, `"home"`, `"end"`, `"page_up"`, `"page_down"`.

use crate::error::InputError;

/// Parsed canonical key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalKey {
    /// Single printable ASCII character (after normalization).
    Char(char),
    /// Named special key.
    Named(NamedKey),
}

/// Named special keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NamedKey {
    /// Return / Enter.
    Return,
    /// Tab.
    Tab,
    /// Spacebar.
    Space,
    /// Escape.
    Escape,
    /// Backspace.
    Backspace,
    /// Forward delete.
    Delete,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Function keys 1..12.
    Function(u8),
}

/// Parse a canonical name. Returns `UnknownKey` on anything we don't recognize.
pub fn parse(name: &str) -> Result<CanonicalKey, InputError> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty() {
        return Err(InputError::UnknownKey(name.to_owned()));
    }

    if let Some(rest) = n.strip_prefix('f') {
        if let Ok(num) = rest.parse::<u8>() {
            if (1..=12).contains(&num) {
                return Ok(CanonicalKey::Named(NamedKey::Function(num)));
            }
        }
    }

    let named = match n.as_str() {
        "return" | "enter" => NamedKey::Return,
        "tab" => NamedKey::Tab,
        "space" => NamedKey::Space,
        "escape" | "esc" => NamedKey::Escape,
        "backspace" => NamedKey::Backspace,
        "delete" | "forward_delete" => NamedKey::Delete,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "page_up" | "pageup" => NamedKey::PageUp,
        "page_down" | "pagedown" => NamedKey::PageDown,
        _ => {
            if n.chars().count() == 1 {
                return Ok(CanonicalKey::Char(n.chars().next().unwrap()));
            }
            return Err(InputError::UnknownKey(name.to_owned()));
        }
    };
    Ok(CanonicalKey::Named(named))
}

/// Translate a parsed key into a macOS virtual keycode (from
/// `HIToolbox/Events.h` / `kVK_*`). Returns `None` for code-points that can't
/// be typed via a virtual key (fallback path: type_text via Unicode events).
#[cfg(target_os = "macos")]
#[must_use]
pub fn macos_keycode(key: CanonicalKey) -> Option<u16> {
    use CanonicalKey::{Char, Named};
    use NamedKey::{
        Backspace, Delete, Down, End, Escape, Function, Home, Left, PageDown, PageUp, Return,
        Right, Space, Tab, Up,
    };
    Some(match key {
        Named(Return) => 0x24,
        Named(Tab) => 0x30,
        Named(Space) => 0x31,
        Named(Backspace) => 0x33,
        Named(Escape) => 0x35,
        Named(Delete) => 0x75,
        Named(Home) => 0x73,
        Named(End) => 0x77,
        Named(PageUp) => 0x74,
        Named(PageDown) => 0x79,
        Named(Left) => 0x7B,
        Named(Right) => 0x7C,
        Named(Down) => 0x7D,
        Named(Up) => 0x7E,
        Named(Function(1)) => 0x7A,
        Named(Function(2)) => 0x78,
        Named(Function(3)) => 0x63,
        Named(Function(4)) => 0x76,
        Named(Function(5)) => 0x60,
        Named(Function(6)) => 0x61,
        Named(Function(7)) => 0x62,
        Named(Function(8)) => 0x64,
        Named(Function(9)) => 0x65,
        Named(Function(10)) => 0x6D,
        Named(Function(11)) => 0x67,
        Named(Function(12)) => 0x6F,
        Named(Function(_)) => return None,
        Char(c) => return ascii_to_macos_keycode(c),
    })
}

#[cfg(target_os = "macos")]
fn ascii_to_macos_keycode(c: char) -> Option<u16> {
    // kVK_ANSI_* keycodes for US layout (HIToolbox/Events.h).
    Some(match c {
        'a' => 0x00, 's' => 0x01, 'd' => 0x02, 'f' => 0x03, 'h' => 0x04, 'g' => 0x05,
        'z' => 0x06, 'x' => 0x07, 'c' => 0x08, 'v' => 0x09, 'b' => 0x0B, 'q' => 0x0C,
        'w' => 0x0D, 'e' => 0x0E, 'r' => 0x0F, 'y' => 0x10, 't' => 0x11,
        '1' => 0x12, '2' => 0x13, '3' => 0x14, '4' => 0x15, '6' => 0x16, '5' => 0x17,
        '=' => 0x18, '9' => 0x19, '7' => 0x1A, '-' => 0x1B, '8' => 0x1C, '0' => 0x1D,
        ']' => 0x1E, 'o' => 0x1F, 'u' => 0x20, '[' => 0x21, 'i' => 0x22, 'p' => 0x23,
        'l' => 0x25, 'j' => 0x26, '\'' => 0x27, 'k' => 0x28, ';' => 0x29, '\\' => 0x2A,
        ',' => 0x2B, '/' => 0x2C, 'n' => 0x2D, 'm' => 0x2E, '.' => 0x2F, '`' => 0x32,
        _ => return None,
    })
}

/// Non-macOS stub so the symbol exists on every platform but always returns None.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn macos_keycode(_key: CanonicalKey) -> Option<u16> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_parse() {
        assert_eq!(parse("return").unwrap(), CanonicalKey::Named(NamedKey::Return));
        assert_eq!(parse("Enter").unwrap(), CanonicalKey::Named(NamedKey::Return));
        assert_eq!(parse("esc").unwrap(), CanonicalKey::Named(NamedKey::Escape));
        assert_eq!(parse("page_up").unwrap(), CanonicalKey::Named(NamedKey::PageUp));
    }

    #[test]
    fn function_keys_parse_in_range() {
        assert_eq!(parse("f1").unwrap(), CanonicalKey::Named(NamedKey::Function(1)));
        assert_eq!(parse("F12").unwrap(), CanonicalKey::Named(NamedKey::Function(12)));
        assert!(matches!(parse("f0"), Err(InputError::UnknownKey(_))));
        assert!(matches!(parse("f13"), Err(InputError::UnknownKey(_))));
    }

    #[test]
    fn single_chars_parse() {
        assert_eq!(parse("s").unwrap(), CanonicalKey::Char('s'));
        assert_eq!(parse("1").unwrap(), CanonicalKey::Char('1'));
    }

    #[test]
    fn empty_and_garbage_reject() {
        assert!(matches!(parse(""), Err(InputError::UnknownKey(_))));
        assert!(matches!(parse("blargh"), Err(InputError::UnknownKey(_))));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_keycode_known_keys() {
        assert_eq!(macos_keycode(parse("return").unwrap()), Some(0x24));
        assert_eq!(macos_keycode(parse("a").unwrap()), Some(0x00));
        assert_eq!(macos_keycode(parse("f1").unwrap()), Some(0x7A));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-input --lib keymap`
Expected: 4 tests pass on non-macOS, 5 on macOS.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-input/src/keymap.rs crates/vcli-input/src/lib.rs
git commit -m "vcli-input: canonical key parser + macOS virtual keycode table"
```

---

### Task 9: Permission diagnostics (TCC probes)

**Files:**
- Create: `crates/vcli-input/src/permissions.rs`
- Create: `crates/vcli-input/src/macos/tcc.rs`
- Create: `crates/vcli-input/src/macos/mod.rs`
- Modify: `crates/vcli-input/src/lib.rs`

- [ ] **Step 1: Add `mod permissions;` + `mod macos;` + re-exports**

Append to `src/lib.rs`:

```rust
pub mod permissions;

pub use permissions::{PermissionStatus, PermissionReport};

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;
```

- [ ] **Step 2: Write `permissions.rs`**

```rust
//! Permission diagnostics. On macOS, input synthesis needs Accessibility AND
//! (for some event types) Input Monitoring TCC buckets granted. Reports a
//! status per bucket without prompting (diagnostic only).

use serde::{Deserialize, Serialize};

/// Status of a single TCC (or equivalent) permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionStatus {
    /// Granted — input synthesis works.
    Granted,
    /// Denied — user explicitly refused.
    Denied,
    /// Not determined — never asked. Typically means granted in practice for
    /// Accessibility if the user has not yet opened the toggle, so dispatch
    /// will prompt / fail the first time.
    NotDetermined,
    /// Platform does not use this concept (always returned on non-macOS).
    NotApplicable,
}

/// Aggregate report printed by `vcli health` and the daemon readiness check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionReport {
    /// macOS Accessibility bucket (required for `CGEventPost`).
    pub accessibility: PermissionStatus,
    /// macOS Input Monitoring bucket (required for the `CGEventTap` listener
    /// used by the kill-switch hotkey).
    pub input_monitoring: PermissionStatus,
}

impl PermissionReport {
    /// True iff both buckets are `Granted`.
    #[must_use]
    pub fn fully_granted(&self) -> bool {
        matches!(self.accessibility, PermissionStatus::Granted)
            && matches!(self.input_monitoring, PermissionStatus::Granted)
    }
}

/// Probe permissions. On macOS this calls into `macos::tcc`; on any other OS
/// both buckets report `NotApplicable`.
#[must_use]
pub fn probe() -> PermissionReport {
    #[cfg(target_os = "macos")]
    {
        crate::macos::tcc::probe_report()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionReport {
            accessibility: PermissionStatus::NotApplicable,
            input_monitoring: PermissionStatus::NotApplicable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_roundtrip() {
        let r = PermissionReport {
            accessibility: PermissionStatus::Granted,
            input_monitoring: PermissionStatus::Denied,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""accessibility":"granted""#));
        assert!(j.contains(r#""input_monitoring":"denied""#));
        let back: PermissionReport = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn fully_granted_requires_both() {
        assert!(PermissionReport {
            accessibility: PermissionStatus::Granted,
            input_monitoring: PermissionStatus::Granted,
        }
        .fully_granted());
        assert!(!PermissionReport {
            accessibility: PermissionStatus::Granted,
            input_monitoring: PermissionStatus::Denied,
        }
        .fully_granted());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_reports_not_applicable() {
        let r = probe();
        assert_eq!(r.accessibility, PermissionStatus::NotApplicable);
        assert_eq!(r.input_monitoring, PermissionStatus::NotApplicable);
    }
}
```

- [ ] **Step 3: Write `src/macos/mod.rs`**

```rust
//! macOS-only backend modules. Gated on `cfg(target_os = "macos")` at the
//! crate root.

pub mod tcc;
pub mod cg_events;
pub mod cg_typing;
pub mod cg_sink;
pub mod hotkey_tap;

pub use cg_sink::CGEventInputSink;
pub use hotkey_tap::{spawn_kill_switch_listener, KillSwitchListenerHandle};
```

- [ ] **Step 4: Write `src/macos/tcc.rs`**

```rust
//! macOS TCC probes. Uses `AXIsProcessTrustedWithOptions` for Accessibility
//! and `IOHIDCheckAccess` for Input Monitoring. No prompts are triggered —
//! the options dictionary passes `kAXTrustedCheckOptionPrompt: false`.

#![allow(unsafe_code)]

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

use crate::permissions::{PermissionReport, PermissionStatus};

extern "C" {
    fn AXIsProcessTrustedWithOptions(options: core_foundation::dictionary::CFDictionaryRef) -> bool;
    fn IOHIDCheckAccess(request: u32) -> u32; // IOHIDRequestType: 0=PostEvent, 1=ListenEvent
}

// Values from IOKit IOHIDLib.h: kIOHIDAccessType{Granted=0,Denied=1,Unknown=2}.
const IOHID_ACCESS_GRANTED: u32 = 0;
const IOHID_ACCESS_DENIED: u32 = 1;
const IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;

/// Full report (Accessibility + Input Monitoring).
#[must_use]
pub fn probe_report() -> PermissionReport {
    PermissionReport {
        accessibility: accessibility_status(),
        input_monitoring: input_monitoring_status(),
    }
}

fn accessibility_status() -> PermissionStatus {
    // Build {"AXTrustedCheckOptionPrompt": false} dictionary without prompting.
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::false_value();
    let dict = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    let trusted = unsafe { AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef()) };
    if trusted {
        PermissionStatus::Granted
    } else {
        // AX API doesn't distinguish Denied vs NotDetermined. Report
        // NotDetermined so the caller prompts the user the first time.
        PermissionStatus::NotDetermined
    }
}

fn input_monitoring_status() -> PermissionStatus {
    let status = unsafe { IOHIDCheckAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT) };
    match status {
        IOHID_ACCESS_GRANTED => PermissionStatus::Granted,
        IOHID_ACCESS_DENIED => PermissionStatus::Denied,
        _ => PermissionStatus::NotDetermined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_does_not_panic() {
        // In CI Input Monitoring will typically be NotDetermined and
        // Accessibility will typically be NotDetermined — we only assert we
        // got SOME PermissionReport value.
        let r = probe_report();
        let _ = serde_json::to_string(&r).unwrap();
    }
}
```

- [ ] **Step 5: Write `src/windows/mod.rs` (stub so cfg gate compiles)**

```rust
//! Windows stub backend. v0.4 replaces this with a real implementation (see
//! spec §Roadmap). Currently all methods return `InputError::Unimplemented`.

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use crate::error::InputError;
use crate::sink::{DragSegment, InputSink};

/// Windows stub sink. Constructible but every method returns `Unimplemented`.
#[derive(Debug, Default)]
pub struct WindowsInputSink;

impl WindowsInputSink {
    /// Constructor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl InputSink for WindowsInputSink {
    fn mouse_move(&self, _to: Point) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn click(&self, _: Point, _: Button, _: &[Modifier], _: u32) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn double_click(&self, _: Point, _: Button) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn drag(&self, _: Point, _: &[DragSegment], _: Button) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn type_text(&self, _: &str) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn key_combo(&self, _: &[Modifier], _: &str) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p vcli-input --lib permissions`
Expected: 2 tests pass on non-macOS (3 including the `non_macos_reports_not_applicable` gate); on macOS, `tcc::probe_does_not_panic` also runs. macOS path links against Core Foundation.

Verify clean build on macOS:
`cargo check -p vcli-input --target aarch64-apple-darwin` (skip if not on macOS).

- [ ] **Step 7: Commit**

```bash
git add crates/vcli-input/src/permissions.rs \
        crates/vcli-input/src/macos/mod.rs \
        crates/vcli-input/src/macos/tcc.rs \
        crates/vcli-input/src/windows/mod.rs \
        crates/vcli-input/src/lib.rs
git commit -m "vcli-input: PermissionReport + macOS TCC probes + Windows stub scaffolding"
```

---

### Task 10: CGEvent low-level helpers (move / click / scroll / key down+up)

**Files:**
- Create: `crates/vcli-input/src/macos/cg_events.rs`

All code in this task is `#[cfg(target_os = "macos")]`.

- [ ] **Step 1: Write `cg_events.rs`**

```rust
//! Raw CGEvent builders. Each function creates an event, posts it to the HID
//! tap, and returns. `CGEventPost` is synchronous — by the time it returns,
//! the OS has enqueued the event on the global event stream (spec §Action
//! confirmation).

#![allow(unsafe_code)]

use std::time::Duration;

use core_foundation::base::CFRelease;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGKeyCode, CGMouseButton,
    EventField, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use crate::error::InputError;

/// Make a new CGEventSource for `HIDSystemState`. Returns `InputError::Backend`
/// on FFI failure.
fn event_source() -> Result<CGEventSource, InputError> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| InputError::Backend { detail: "CGEventSource::new failed".into() })
}

/// Convert a Point to CGPoint in logical display coordinates.
#[must_use]
pub fn to_cg(p: Point) -> CGPoint {
    CGPoint::new(f64::from(p.x), f64::from(p.y))
}

/// Translate modifiers to a CGEventFlags bitmask.
#[must_use]
pub fn flags_from_modifiers(modifiers: &[Modifier]) -> CGEventFlags {
    let mut f = CGEventFlags::empty();
    for m in modifiers {
        f |= match m {
            Modifier::Cmd => CGEventFlags::CGEventFlagCommand,
            Modifier::Shift => CGEventFlags::CGEventFlagShift,
            Modifier::Alt => CGEventFlags::CGEventFlagAlternate,
            Modifier::Ctrl => CGEventFlags::CGEventFlagControl,
        };
    }
    f
}

/// Translate our Button to CGMouseButton + the (type_down, type_up) pair we
/// need to post for that button.
#[must_use]
pub fn button_types(b: Button) -> (CGMouseButton, CGEventType, CGEventType) {
    match b {
        Button::Left => (
            CGMouseButton::Left,
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
        ),
        Button::Right => (
            CGMouseButton::Right,
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
        ),
        Button::Middle => (
            CGMouseButton::Center,
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
        ),
    }
}

/// Post a `MouseMoved` event to the HID tap.
pub fn post_move(to: Point) -> Result<(), InputError> {
    let src = event_source()?;
    let event = CGEvent::new_mouse_event(
        src,
        CGEventType::MouseMoved,
        to_cg(to),
        CGMouseButton::Left, // ignored for MouseMoved
    )
    .map_err(|()| InputError::Backend { detail: "CGEvent::new_mouse_event MouseMoved failed".into() })?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

/// Post a down-then-up click, setting `click_state` (`MouseEventClickState`
/// field, integer 1 for single-click, 2 for double-click, etc.) and modifier
/// flags.
pub fn post_click(
    at: Point,
    button: Button,
    modifiers: &[Modifier],
    hold: Duration,
    click_state: i64,
) -> Result<(), InputError> {
    let src = event_source()?;
    let (cg_btn, down_ty, up_ty) = button_types(button);
    let flags = flags_from_modifiers(modifiers);

    // Down.
    let down = CGEvent::new_mouse_event(src.clone(), down_ty, to_cg(at), cg_btn)
        .map_err(|()| InputError::Backend { detail: "mouse_event down failed".into() })?;
    down.set_flags(flags);
    down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    down.post(CGEventTapLocation::HID);

    if !hold.is_zero() {
        std::thread::sleep(hold);
    }

    // Up.
    let up = CGEvent::new_mouse_event(src, up_ty, to_cg(at), cg_btn)
        .map_err(|()| InputError::Backend { detail: "mouse_event up failed".into() })?;
    up.set_flags(flags);
    up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

/// Post a dragged mouse motion (button already down) to `to`.
pub fn post_drag_move(to: Point, button: Button) -> Result<(), InputError> {
    let src = event_source()?;
    let drag_ty = match button {
        Button::Left => CGEventType::LeftMouseDragged,
        Button::Right => CGEventType::RightMouseDragged,
        Button::Middle => CGEventType::OtherMouseDragged,
    };
    let cg_btn = button_types(button).0;
    let ev = CGEvent::new_mouse_event(src, drag_ty, to_cg(to), cg_btn)
        .map_err(|()| InputError::Backend { detail: "mouse_event drag failed".into() })?;
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

/// Post a key down or up event for a virtual keycode.
pub fn post_key(
    keycode: CGKeyCode,
    key_down: bool,
    modifiers: &[Modifier],
) -> Result<(), InputError> {
    let src = event_source()?;
    let ev = CGEvent::new_keyboard_event(src, keycode, key_down)
        .map_err(|()| InputError::Backend { detail: "CGEvent::new_keyboard_event failed".into() })?;
    ev.set_flags(flags_from_modifiers(modifiers));
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

/// Post a scroll-wheel event with pixel units.
pub fn post_scroll(_at: Point, dx: i32, dy: i32) -> Result<(), InputError> {
    // Scroll events aren't positional on macOS (they go to the focused window),
    // but we keep the same signature as `InputSink` for symmetry.
    let src = event_source()?;
    let ev = CGEvent::new_scroll_event(src, ScrollEventUnit::PIXEL, 2, dy, dx, 0)
        .map_err(|()| InputError::Backend { detail: "CGEvent::new_scroll_event failed".into() })?;
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

// No tests here: every function talks to the OS. Exercised by
// `tests/macos_real.rs` behind `#[ignore]`.
```

> **Note for the implementer:** the exact API of the `core-graphics` crate at
> version `0.24` may rename these items (e.g. `EventField::MOUSE_EVENT_CLICK_STATE`
> vs `CGEventField::mouseEventClickState`). If names diverge, run
> `cargo doc --open -p core-graphics` and adjust the imports — the behavior
> contract stays the same.

- [ ] **Step 2: Verify it compiles on macOS**

Run (from a macOS host): `cargo check -p vcli-input`
Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-input/src/macos/cg_events.rs
git commit -m "vcli-input: macOS CGEvent helpers (move/click/drag/key/scroll)"
```

---

### Task 11: Unicode typing helper

**Files:**
- Create: `crates/vcli-input/src/macos/cg_typing.rs`

- [ ] **Step 1: Write `cg_typing.rs`**

```rust
//! `type_text` for macOS. Uses `CGEventKeyboardSetUnicodeString` so the active
//! keyboard layout is respected and every Unicode code point is typeable.

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use core_graphics::event::{CGEvent, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

use crate::error::InputError;

/// Post one keyboard event carrying `chunk` as its Unicode payload (key down)
/// followed by a matching key up. Called once per grapheme cluster so layouts
/// that render combining marks still produce the expected glyph.
fn post_unicode_chunk(chunk: &str) -> Result<(), InputError> {
    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| InputError::Backend { detail: "CGEventSource::new failed".into() })?;

    let utf16: Vec<u16> = chunk.encode_utf16().collect();

    let down = CGEvent::new_keyboard_event(src.clone(), 0, true)
        .map_err(|()| InputError::Backend { detail: "keyboard_event down failed".into() })?;
    down.set_string_from_utf16_unchecked(&utf16);
    down.post(CGEventTapLocation::HID);

    let up = CGEvent::new_keyboard_event(src, 0, false)
        .map_err(|()| InputError::Backend { detail: "keyboard_event up failed".into() })?;
    up.set_string_from_utf16_unchecked(&utf16);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

/// Type a UTF-8 string one grapheme cluster at a time. If the `core-graphics`
/// crate lacks a helper with that exact name, the implementer should use
/// `set_string`, `set_string_from_utf16`, or the FFI `CGEventKeyboardSetUnicodeString`
/// directly.
pub fn type_text(text: &str) -> Result<(), InputError> {
    if text.is_empty() {
        return Ok(());
    }
    // One event per grapheme cluster. v0 approximation: one per character.
    for ch in text.chars() {
        let mut buf = [0u8; 4];
        let chunk = ch.encode_utf8(&mut buf);
        post_unicode_chunk(chunk)?;
    }
    Ok(())
}
```

- [ ] **Step 2: Verify it compiles on macOS**

Run (on macOS): `cargo check -p vcli-input`
Expected: builds clean. If `set_string_from_utf16_unchecked` is not exposed by `core-graphics 0.24`, replace with a direct FFI call:

```rust
extern "C" {
    fn CGEventKeyboardSetUnicodeString(event: core_graphics::sys::CGEventRef, length: usize, string: *const u16);
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-input/src/macos/cg_typing.rs
git commit -m "vcli-input: macOS Unicode typing helper (CGEventKeyboardSetUnicodeString)"
```

---

### Task 12: `CGEventInputSink` — wire helpers into the `InputSink` trait

**Files:**
- Create: `crates/vcli-input/src/macos/cg_sink.rs`

- [ ] **Step 1: Write `cg_sink.rs`**

```rust
//! Real macOS `InputSink`. Enforces the [`KillSwitch`] on every entry point,
//! then delegates to the low-level CGEvent helpers.

#![cfg(target_os = "macos")]

use std::time::Duration;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use super::{cg_events, cg_typing};
use crate::error::InputError;
use crate::keymap::{parse, CanonicalKey, macos_keycode};
use crate::kill_switch::KillSwitch;
use crate::permissions::{probe, PermissionStatus};
use crate::sink::{DragSegment, InputSink};

/// Maximum step interpolation time for a single drag segment. Guards against
/// runaway durations from ill-formed programs.
const MAX_DRAG_SEGMENT_MS: u64 = 5_000;
/// Pixel interval between interpolated drag-move events.
const DRAG_STEP_PX: i32 = 8;

/// macOS `CGEvent`-backed `InputSink`.
#[derive(Debug)]
pub struct CGEventInputSink {
    kill: KillSwitch,
}

impl CGEventInputSink {
    /// Construct with a caller-provided kill switch. Callers should also spawn
    /// the hotkey listener via [`super::spawn_kill_switch_listener`] so the
    /// `Cmd+Shift+Esc` chord engages this switch.
    #[must_use]
    pub fn new(kill: KillSwitch) -> Self {
        Self { kill }
    }

    /// Fail-fast if Accessibility isn't granted. Called on first `InputSink`
    /// method invocation only when we know we'd hit the OS. Cheap enough
    /// (single `AXIsProcessTrustedWithOptions` call) to gate every call.
    fn guard(&self) -> Result<(), InputError> {
        if self.kill.is_engaged() {
            return Err(InputError::Halted);
        }
        let report = probe();
        if !matches!(report.accessibility, PermissionStatus::Granted | PermissionStatus::NotDetermined) {
            return Err(InputError::PermissionDenied {
                detail: "Accessibility (TCC) not granted".into(),
            });
        }
        Ok(())
    }
}

impl InputSink for CGEventInputSink {
    fn mouse_move(&self, to: Point) -> Result<(), InputError> {
        self.guard()?;
        cg_events::post_move(to)
    }

    fn click(
        &self,
        at: Point,
        button: Button,
        modifiers: &[Modifier],
        hold_ms: u32,
    ) -> Result<(), InputError> {
        self.guard()?;
        cg_events::post_move(at)?;
        cg_events::post_click(at, button, modifiers, Duration::from_millis(hold_ms.into()), 1)
    }

    fn double_click(&self, at: Point, button: Button) -> Result<(), InputError> {
        self.guard()?;
        cg_events::post_move(at)?;
        cg_events::post_click(at, button, &[], Duration::ZERO, 1)?;
        cg_events::post_click(at, button, &[], Duration::ZERO, 2)
    }

    fn drag(
        &self,
        from: Point,
        segments: &[DragSegment],
        button: Button,
    ) -> Result<(), InputError> {
        self.guard()?;
        if segments.is_empty() {
            return Err(InputError::InvalidArgument("drag segments must be non-empty".into()));
        }

        // Move to start, press down.
        cg_events::post_move(from)?;
        let (_cg_btn, down_ty, up_ty) = cg_events::button_types(button);
        let src = core_graphics::event_source::CGEventSource::new(
            core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
        )
        .map_err(|()| InputError::Backend { detail: "CGEventSource::new failed".into() })?;
        let down = core_graphics::event::CGEvent::new_mouse_event(
            src,
            down_ty,
            cg_events::to_cg(from),
            cg_events::button_types(button).0,
        )
        .map_err(|()| InputError::Backend { detail: "mouse_event down failed".into() })?;
        down.post(core_graphics::event::CGEventTapLocation::HID);

        // Interpolate through each segment.
        let mut current = from;
        for seg in segments {
            if seg.duration.as_millis() > u128::from(MAX_DRAG_SEGMENT_MS) {
                return Err(InputError::InvalidArgument(
                    "drag segment longer than 5s".into(),
                ));
            }
            let dx = seg.to.x - current.x;
            let dy = seg.to.y - current.y;
            let dist = ((dx * dx + dy * dy) as f64).sqrt() as i32;
            let steps = (dist / DRAG_STEP_PX).max(1);
            let sleep_each = seg.duration / u32::try_from(steps).unwrap_or(1);
            for i in 1..=steps {
                if self.kill.is_engaged() {
                    // Release button before bailing.
                    let _ = cg_events::post_drag_move(current, button);
                    let src2 = core_graphics::event_source::CGEventSource::new(
                        core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
                    )
                    .ok();
                    if let Some(src) = src2 {
                        if let Ok(up) = core_graphics::event::CGEvent::new_mouse_event(
                            src,
                            up_ty,
                            cg_events::to_cg(current),
                            cg_events::button_types(button).0,
                        ) {
                            up.post(core_graphics::event::CGEventTapLocation::HID);
                        }
                    }
                    return Err(InputError::Halted);
                }
                let nx = current.x + dx * i / steps;
                let ny = current.y + dy * i / steps;
                cg_events::post_drag_move(Point { x: nx, y: ny }, button)?;
                if !sleep_each.is_zero() {
                    std::thread::sleep(sleep_each);
                }
            }
            current = seg.to;
        }

        // Release button at final position.
        let src = core_graphics::event_source::CGEventSource::new(
            core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
        )
        .map_err(|()| InputError::Backend { detail: "CGEventSource::new failed".into() })?;
        let up = core_graphics::event::CGEvent::new_mouse_event(
            src,
            up_ty,
            cg_events::to_cg(current),
            cg_events::button_types(button).0,
        )
        .map_err(|()| InputError::Backend { detail: "mouse_event up failed".into() })?;
        up.post(core_graphics::event::CGEventTapLocation::HID);
        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<(), InputError> {
        self.guard()?;
        cg_typing::type_text(text)
    }

    fn key_combo(&self, modifiers: &[Modifier], key: &str) -> Result<(), InputError> {
        self.guard()?;
        let parsed = parse(key)?;
        let Some(keycode) = macos_keycode(parsed) else {
            return Err(InputError::UnknownKey(key.to_owned()));
        };
        // Press modifiers down-in-order, press key, release key, release modifiers
        // in reverse order. Using CGEventFlags on the key event alone is usually
        // enough, but explicit down/up events are more reliable for apps that
        // read flagChanged events.
        cg_events::post_key(keycode, true, modifiers)?;
        cg_events::post_key(keycode, false, modifiers)?;
        Ok(())
    }
}
```

- [ ] **Step 2: Verify it compiles on macOS**

Run (on macOS): `cargo check -p vcli-input`
Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-input/src/macos/cg_sink.rs
git commit -m "vcli-input: CGEventInputSink wires CGEvent helpers into InputSink"
```

---

### Task 13: Kill-switch hotkey listener (`Cmd+Shift+Esc`)

**Files:**
- Create: `crates/vcli-input/src/macos/hotkey_tap.rs`

- [ ] **Step 1: Write `hotkey_tap.rs`**

```rust
//! CGEventTap-based global hotkey listener.
//!
//! Spawns a dedicated thread with its own `CFRunLoop`. Installs an event tap at
//! `kCGHIDEventTap` in `kCGEventTapOptionListenOnly` mode so it observes — but
//! never consumes — keystrokes. When the `Cmd+Shift+Esc` chord is detected
//! (keycode `0x35` with Cmd + Shift flags on KeyDown), it calls
//! `KillSwitch::engage()`. Dropping the returned `KillSwitchListenerHandle`
//! stops the run loop and joins the thread.
//!
//! Why `Cmd+Shift+Esc`? macOS reserves `Cmd+Option+Esc` for Force Quit;
//! `Cmd+Shift+Esc` is otherwise unused system-wide, easy to chord one-handed,
//! and semantically "escape from the automation."

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use std::ptr;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use core_foundation::base::{CFRelease, TCFType};
use core_foundation::runloop::{
    kCFRunLoopCommonModes, CFRunLoop, CFRunLoopAddSource, CFRunLoopRunInMode,
    CFRunLoopStop,
};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};
use core_graphics::sys::CGEventRef;

use crate::error::InputError;
use crate::kill_switch::KillSwitch;

/// `kVK_Escape` from HIToolbox/Events.h.
const KVK_ESCAPE: u16 = 0x35;

/// Handle returned by [`spawn_kill_switch_listener`]. Dropping stops the tap.
pub struct KillSwitchListenerHandle {
    stop: Arc<Mutex<Option<CFRunLoop>>>,
    join: Option<JoinHandle<()>>,
}

impl Drop for KillSwitchListenerHandle {
    fn drop(&mut self) {
        if let Some(rl) = self.stop.lock().unwrap().take() {
            unsafe { CFRunLoopStop(rl.as_concrete_TypeRef()) };
        }
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

struct CallbackCtx {
    kill: KillSwitch,
}

extern "C" fn callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    ctx_ptr: *mut std::ffi::c_void,
) -> CGEventRef {
    if event_type == CGEventType::KeyDown {
        let ctx: &CallbackCtx = unsafe { &*(ctx_ptr.cast::<CallbackCtx>()) };
        // Safety: the event reference is only used for the lifetime of the callback.
        let wrapper: CGEvent = unsafe { TCFType::wrap_under_get_rule(event) };
        let keycode = wrapper.get_integer_value_field(
            core_graphics::event::EventField::KEYBOARD_EVENT_KEYCODE,
        );
        let flags = wrapper.get_flags();
        let want = CGEventFlags::CGEventFlagCommand | CGEventFlags::CGEventFlagShift;
        if keycode == i64::from(KVK_ESCAPE) && flags.contains(want) {
            ctx.kill.engage();
        }
    }
    // Listen-only mode: the returned reference is ignored. Return the incoming
    // pointer unchanged so we never accidentally drop/consume it.
    event
}

/// Start the listener. Returns a handle — dropping the handle stops the thread.
///
/// Fails if the current process doesn't hold Input Monitoring permission.
pub fn spawn_kill_switch_listener(
    kill: KillSwitch,
) -> Result<KillSwitchListenerHandle, InputError> {
    let stop: Arc<Mutex<Option<CFRunLoop>>> = Arc::new(Mutex::new(None));
    let stop_for_thread = stop.clone();

    let join = std::thread::Builder::new()
        .name("vcli-input-killswitch-tap".into())
        .spawn(move || {
            // Allocate the callback context on the heap so it outlives this scope.
            let ctx = Box::into_raw(Box::new(CallbackCtx { kill }));

            let mask = 1u64 << (CGEventType::KeyDown as u64);

            // Safety: CGEventTapCreate signature from CoreGraphics.
            extern "C" {
                fn CGEventTapCreate(
                    tap: u32,
                    place: u32,
                    options: u32,
                    events_of_interest: u64,
                    callback: extern "C" fn(CGEventTapProxy, CGEventType, CGEventRef, *mut std::ffi::c_void) -> CGEventRef,
                    user_info: *mut std::ffi::c_void,
                ) -> core_foundation::mach_port::CFMachPortRef;
                fn CFMachPortCreateRunLoopSource(
                    allocator: *const std::ffi::c_void,
                    port: core_foundation::mach_port::CFMachPortRef,
                    order: isize,
                ) -> core_foundation::runloop::CFRunLoopSourceRef;
            }

            let port = unsafe {
                CGEventTapCreate(
                    CGEventTapLocation::HID as u32,
                    CGEventTapPlacement::HeadInsertEventTap as u32,
                    CGEventTapOptions::ListenOnly as u32,
                    mask,
                    callback,
                    ctx.cast(),
                )
            };
            if port.is_null() {
                // Input Monitoring not granted (or Accessibility not granted —
                // CGEventTap needs one of them). Drop the context, exit.
                unsafe { drop(Box::from_raw(ctx)) };
                return;
            }

            let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), port, 0) };
            let runloop = CFRunLoop::get_current();
            unsafe {
                CFRunLoopAddSource(runloop.as_concrete_TypeRef(), source, kCFRunLoopCommonModes);
            }
            *stop_for_thread.lock().unwrap() = Some(runloop.clone());

            // Run until CFRunLoopStop.
            loop {
                let code = unsafe {
                    CFRunLoopRunInMode(kCFRunLoopCommonModes, 0.5, 0)
                };
                // Return code 2 = Stopped.
                if code == 2 {
                    break;
                }
                if stop_for_thread.lock().unwrap().is_none() {
                    break;
                }
            }

            unsafe {
                CFRelease(source.cast());
                CFRelease(port.cast());
                drop(Box::from_raw(ctx));
            }
        })
        .map_err(|e| InputError::Backend { detail: format!("spawn tap thread: {e}") })?;

    Ok(KillSwitchListenerHandle { stop, join: Some(join) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawn + drop should not panic; in CI the tap creation will fail cleanly
    /// because the process lacks Input Monitoring, and the thread exits early.
    #[test]
    fn spawn_and_drop_does_not_panic() {
        let kill = KillSwitch::new();
        let handle = spawn_kill_switch_listener(kill.clone()).unwrap();
        drop(handle);
    }
}
```

> **FFI notes for the implementer:** `core-graphics = 0.24` may already expose
> `CGEventTapCreate` safely — prefer the safe wrapper if it exists. The extern
> signatures above are a fallback if the safe wrapper is missing. Verify the
> `CFRunLoopRunInMode` return-code constant against `CFRunLoop.h` (should be
> `kCFRunLoopRunStopped = 2`).

- [ ] **Step 2: Verify it compiles on macOS**

Run (on macOS): `cargo test -p vcli-input --lib macos::hotkey_tap`
Expected: 1 test passes; clean exit with no panic.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-input/src/macos/hotkey_tap.rs
git commit -m "vcli-input: macOS CGEventTap listener for Cmd+Shift+Esc kill switch chord"
```

---

### Task 14: Mock-vs-trait contract test

**Files:**
- Create: `crates/vcli-input/tests/mock_contract.rs`

- [ ] **Step 1: Write `tests/mock_contract.rs`**

```rust
//! Contract tests that run against [`MockInputSink`] and also serve as a
//! template for asserting any future `InputSink` impl.

use std::time::Duration;

use vcli_core::action::{Button, InputAction, Modifier};
use vcli_core::geom::Point;

use vcli_input::error::InputError;
use vcli_input::kill_switch::KillSwitch;
use vcli_input::mock::{MockCall, MockInputSink};
use vcli_input::sink::{DragSegment, InputSink};

fn new_mock() -> MockInputSink {
    MockInputSink::new()
}

#[test]
fn mouse_move_records_once() {
    let m = new_mock();
    m.mouse_move(Point { x: 100, y: 200 }).unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::Action(InputAction::Move {
            at: Point { x: 100, y: 200 }
        })]
    );
}

#[test]
fn click_preserves_modifiers_and_hold() {
    let m = new_mock();
    m.click(
        Point { x: 5, y: 6 },
        Button::Right,
        &[Modifier::Cmd, Modifier::Ctrl],
        75,
    )
    .unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::ClickDetailed {
            at: Point { x: 5, y: 6 },
            button: Button::Right,
            modifiers: vec![Modifier::Cmd, Modifier::Ctrl],
            hold_ms: 75,
        }]
    );
}

#[test]
fn double_click_emits_distinct_variant() {
    let m = new_mock();
    m.double_click(Point { x: 1, y: 1 }, Button::Left).unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::DoubleClick {
            at: Point { x: 1, y: 1 },
            button: Button::Left
        }]
    );
}

#[test]
fn drag_with_multiple_segments_records_all_endpoints() {
    let m = new_mock();
    m.drag(
        Point { x: 0, y: 0 },
        &[
            DragSegment { to: Point { x: 10, y: 10 }, duration: Duration::from_millis(5) },
            DragSegment { to: Point { x: 20, y: 20 }, duration: Duration::from_millis(5) },
        ],
        Button::Left,
    )
    .unwrap();
    match &m.calls()[0] {
        MockCall::Drag { from, to, button } => {
            assert_eq!(*from, Point { x: 0, y: 0 });
            assert_eq!(to, &vec![Point { x: 10, y: 10 }, Point { x: 20, y: 20 }]);
            assert_eq!(*button, Button::Left);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn type_text_records_input_action() {
    let m = new_mock();
    m.type_text("hello 世界").unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::Action(InputAction::Type {
            text: "hello 世界".into()
        })]
    );
}

#[test]
fn key_combo_records_modifiers() {
    let m = new_mock();
    m.key_combo(&[Modifier::Cmd, Modifier::Shift], "s").unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::Action(InputAction::Key {
            key: "s".into(),
            modifiers: vec![Modifier::Cmd, Modifier::Shift],
        })]
    );
}

#[test]
fn empty_drag_is_rejected() {
    let m = new_mock();
    let e = m.drag(Point { x: 0, y: 0 }, &[], Button::Left).unwrap_err();
    assert!(matches!(e, InputError::InvalidArgument(_)));
}

#[test]
fn forced_error_bubbles_as_backend_failure() {
    let m = MockInputSink::new();
    m.fail_with("os returned -1");
    let e = m.type_text("nope").unwrap_err();
    assert!(matches!(e, InputError::Backend { .. }));
}

#[test]
fn kill_switch_engaged_halts_every_method() {
    let kill = KillSwitch::new();
    let m = MockInputSink::with_kill_switch(kill.clone());
    kill.engage();
    assert!(matches!(m.mouse_move(Point { x: 0, y: 0 }).unwrap_err(), InputError::Halted));
    assert!(matches!(m.type_text("x").unwrap_err(), InputError::Halted));
    assert!(matches!(m.key_combo(&[], "a").unwrap_err(), InputError::Halted));
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p vcli-input --test mock_contract`
Expected: 9 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-input/tests/mock_contract.rs
git commit -m "vcli-input: mock-vs-trait contract tests"
```

---

### Task 15: `#[ignore]` integration tests against the real macOS sink

**Files:**
- Create: `crates/vcli-input/tests/macos_real.rs`

- [ ] **Step 1: Write `tests/macos_real.rs`**

```rust
//! Real CGEvent smoke tests. Gated with `#[ignore]` because they move the
//! cursor and need TCC Accessibility (+ optionally Input Monitoring) granted.
//!
//! Run manually with:
//!     cargo test -p vcli-input --test macos_real -- --ignored
//!
//! If Accessibility is not granted, every test will produce
//! `InputError::PermissionDenied` and fail fast with a clear message.

#![cfg(target_os = "macos")]

use std::time::Duration;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use vcli_input::kill_switch::KillSwitch;
use vcli_input::macos::CGEventInputSink;
use vcli_input::permissions::{probe, PermissionStatus};
use vcli_input::sink::{DragSegment, InputSink};

fn require_accessibility() -> Result<(), String> {
    let r = probe();
    if matches!(r.accessibility, PermissionStatus::Granted) {
        Ok(())
    } else {
        Err(format!("Accessibility TCC not granted: {r:?}"))
    }
}

fn sink() -> CGEventInputSink {
    CGEventInputSink::new(KillSwitch::new())
}

#[test]
#[ignore = "moves the cursor; requires TCC Accessibility"]
fn mouse_move_to_origin_and_back() {
    require_accessibility().unwrap();
    let s = sink();
    s.mouse_move(Point { x: 50, y: 50 }).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    s.mouse_move(Point { x: 200, y: 200 }).unwrap();
}

#[test]
#[ignore = "clicks the screen; requires TCC Accessibility"]
fn click_left_at_a_safe_spot() {
    require_accessibility().unwrap();
    let s = sink();
    // (10, 10) is usually over the menu bar / desktop; harmless.
    s.click(Point { x: 10, y: 10 }, Button::Left, &[], 20).unwrap();
}

#[test]
#[ignore = "types into the focused window; requires TCC Accessibility"]
fn type_ascii_text() {
    require_accessibility().unwrap();
    let s = sink();
    s.type_text("hello").unwrap();
}

#[test]
#[ignore = "presses Cmd+A in the focused window; requires TCC Accessibility"]
fn key_combo_cmd_a() {
    require_accessibility().unwrap();
    let s = sink();
    s.key_combo(&[Modifier::Cmd], "a").unwrap();
}

#[test]
#[ignore = "drags across 100 pixels; requires TCC Accessibility"]
fn drag_100_pixels() {
    require_accessibility().unwrap();
    let s = sink();
    s.drag(
        Point { x: 100, y: 100 },
        &[DragSegment { to: Point { x: 200, y: 200 }, duration: Duration::from_millis(200) }],
        Button::Left,
    )
    .unwrap();
}

#[test]
#[ignore = "verifies kill switch engagement short-circuits real sink"]
fn kill_switch_short_circuits_real_sink() {
    require_accessibility().unwrap();
    let kill = KillSwitch::new();
    let s = CGEventInputSink::new(kill.clone());
    kill.engage();
    let e = s.mouse_move(Point { x: 100, y: 100 }).unwrap_err();
    assert!(matches!(e, vcli_input::error::InputError::Halted));
}
```

- [ ] **Step 2: Verify the tests at least compile**

Run (on macOS): `cargo test -p vcli-input --test macos_real -- --list`
Expected: all 6 tests listed, none run (all `#[ignore]`).

Run (on any OS): `cargo check -p vcli-input --tests`
Expected: clean build (non-macOS skips `macos_real.rs` via `#[cfg(target_os = "macos")]`).

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-input/tests/macos_real.rs
git commit -m "vcli-input: #[ignore] CGEvent integration smoke tests"
```

---

### Task 16: Full-crate verification

**Files:** (none new — verification pass)

- [ ] **Step 1: Run the full test suite (non-macOS-gated)**

Run: `cargo test -p vcli-input`
Expected: on Linux CI, runs ~28 unit tests + ~13 integration tests (mock contract + kill switch), all pass. On macOS, additionally runs `macos::hotkey_tap::spawn_and_drop_does_not_panic` and the tcc probe smoke test.

- [ ] **Step 2: Run clippy in pedantic mode**

Run: `cargo clippy -p vcli-input --all-targets -- -D warnings`
Expected: no warnings, no errors. (If macOS-only unsafe blocks trigger lints, suppress narrowly with `#[allow(...)]` adjacent to the call site and note the reason in a comment — never `#![allow]` at the crate level for unsafe-related lints.)

- [ ] **Step 3: Run rustfmt check**

Run: `cargo fmt --all -- --check`
Expected: no diff.

- [ ] **Step 4: Verify docs build clean**

Run: `cargo doc -p vcli-input --no-deps`
Expected: builds; `#![warn(missing_docs)]` enforces rustdoc coverage.

- [ ] **Step 5: Run the full ignored integration suite manually on a macOS host with TCC granted**

Run: `cargo test -p vcli-input --test macos_real -- --ignored --test-threads=1`
Expected (on a correctly-configured macOS host): 6 tests pass, cursor visibly moves, focused window receives typed text. If any test fails with `PermissionDenied`, the tester needs to grant Accessibility in System Settings → Privacy & Security.

- [ ] **Step 6: Final commit (empty verify commit)**

If tasks 1-15 each already committed, no new commit is needed. Otherwise:

```bash
git commit --allow-empty -m "vcli-input: Phase complete — clippy + fmt + docs green"
```

- [ ] **Step 7: Tag the milestone**

```bash
git tag lane-d-vcli-input-complete -m "vcli-input complete — InputSink, CGEvent backend, kill switch, mock"
```

---

## What this plan unlocks

With `vcli-input` landed, `vcli-runtime` can build its input-dispatch path against `InputSink` + `KillSwitch` without waiting for the real macOS backend (runs on Linux CI against the mock). Scenario tests in `vcli-runtime` compose `MockInputSink` with `CannedSequenceCapture` (Lane C) to produce fully-deterministic action/postcondition traces, and the `vcli-daemon` (future lane) wires `spawn_kill_switch_listener` into its startup so humans always have a hardware-chord escape hatch.

---

## Self-review

- **Spec coverage.** Every primitive required by the lane brief has a task:
  scaffolding (Tasks 1–2), error type (Task 3), `KillSwitch` atomic flag + observer (Task 4), `InputSink` trait (Task 5), `MockInputSink` recording `InputAction` log (Task 6), `KillSwitch` integration test (Task 7), key-name → virtual-keycode table (Task 8), permission diagnostics for Accessibility + Input Monitoring (Task 9), macOS CGEvent impls for move/click/drag/type_text/key_combo (Tasks 10–12), CGEventTap hotkey listener (Task 13), mock contract tests (Task 14), `#[ignore]` real-sink integration tests (Task 15), full verification (Task 16). Windows stub is produced in Task 9 (`src/windows/mod.rs`) via `unimplemented!()`. DO-NOT-INCLUDE list respected: no arbitration / scheduling (runtime lane), no IPC (Lane E), no store (Lane F).
- **TDD discipline.** Every task with new logic writes tests first, then the minimum code to pass, then commits. Tasks that only wire FFI (10, 11, 13) deliberately defer tests to the `#[ignore]` integration suite and name the fact explicitly — matching the reference plan's approach of gating real-hardware tests behind `#[ignore]`.
- **Bite-sized steps.** Each checkbox is a 2–5 minute action with real code, no TODOs.
- **Commit hygiene.** Every task ends with `git add <exact files>` and a `vcli-input: …` commit message.
- **Kill-switch semantics match spec.** `InputSink` methods return `InputError::Halted` synchronously when engaged (mock test in Task 7; real-sink test in Task 15). The listener is off-tick-loop (its own `CFRunLoop` thread) so it can't starve perception.
- **macOS-first + Windows abstraction.** The trait is platform-agnostic; the macOS impl is behind `#[cfg(target_os = "macos")]`; the Windows path in `src/windows/mod.rs` gives a compilable `unimplemented!()` for when `cargo check --target x86_64-pc-windows-msvc` lands.
- **Dependency on `vcli-core` only.** No changes proposed to `vcli-core`; the plan pulls `InputAction`, `Button`, `Modifier`, `Point` as-is.
- **Decision references.** Ties to spec §Action confirmation (synchronous dispatch), Codex Decision B (HIL kill switch), Decision F1 (logical-pixel coordinates — every `Point` in this crate is logical), Decision 2.1 (`thiserror` in library crates). No resurfaced / contradicted decisions.
- **CI-friendly.** Non-macOS builds compile and test green because every macOS module is `#[cfg(target_os = "macos")]`-gated and the Windows path uses `#[cfg(target_os = "windows")]`; neither is required for CI's ubuntu job.

---

## Reporting back

(1) **Path written:** plan content delivered inline above for `docs/superpowers/plans/2026-04-16-vcli-input.md`. I operate in read-only planning mode and cannot create files on disk; the parent agent (or the user) must persist this content at that path.

(2) **Total task count:** 16 tasks.

(3) **CGEvent / hotkey crate choice:** `core-graphics = "0.24"` (with `core-foundation = "0.10"` for `CFRunLoop` / `CFMachPort` glue and a minimal `extern "C"` fallback for `CGEventTapCreate` in case the safe wrapper isn't exposed in that version). Rationale: the `core-graphics` crate is already the lingua franca of Rust-on-macOS in this workspace (the capture lane also uses Core Graphics types for display geometry), gives direct access to `CGEventField` / `MouseEventClickState` / `KeyboardEventKeycode` which we need for deterministic double-clicks and flag-preserving key events, and keeps us on one FFI dependency instead of pulling in `enigo` (which papers over those fields). The hotkey listener uses `CGEventTapCreate` on the same library — no second FFI crate.

(4) **Kill-switch chord:** `Cmd+Shift+Esc`. Rationale: (a) `Cmd+Option+Esc` is reserved by macOS for Force Quit — avoiding confusion with that is critical; (b) `Cmd+Shift+Esc` has no system-wide binding on macOS, so installing a listen-only tap on it won't conflict with anything; (c) the mnemonic is obvious ("escape from the automation"); (d) the chord is easy to hit one-handed in a panic; (e) requiring two modifiers prevents accidental fat-fingering of `Esc` alone from engaging the kill switch. The listener runs in `kCGEventTapOptionListenOnly` mode so the chord still reaches any focused app that may have its own binding for it.
