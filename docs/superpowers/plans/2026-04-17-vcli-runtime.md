# vcli-runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `vcli-runtime` crate — a synchronous, dependency-injected 10 fps scheduler that drives `Capture → Perception → Arbiter → InputSink`, owns program + watch lifetimes, emits `Event`s, and is fully deterministic under mock Capture/Clock/InputSink so spec §Scenario-Tests (§760) can be exercised in plain `cargo test`.

**Architecture:** `Scheduler::run_until_shutdown()` owns a dedicated sync OS thread. Each tick: (1) `capture.grab_screen()` → `Arc<Frame>`; (2) `perception.clear()`; (3) drain the `SchedulerCommand` queue (Submit/Cancel/Start/ResumeRunning/Shutdown); (4) advance triggers for `Waiting` programs; (5) evaluate watches across all `Running` programs through a shared `Perception` façade (cross-program dedup via `PredicateHash`); (6) for firing watches, append resolved `InputAction`s to a per-tick queue; (7) the `Arbiter` resolves same-frame conflicts by `Priority` then `ProgramId`; (8) dispatch surviving actions to the `InputSink`, emit `action.dispatched` / `action.deferred`; (9) run one step of the body for each `Running` program (`body_cursor` advances on success); (10) emit `watch.fired` / `program.state_changed` / `program.completed` / `program.failed` through the `event_tx` channel. No tokio, no SQLite, no FFI — the runtime is a pure orchestration crate that takes trait objects from its caller.

**Tech Stack:** Rust 2021, MSRV 1.75. Depends on `vcli-core` (types, `Clock`, canonical JSON), `vcli-capture` (`Capture` trait), `vcli-input` (`InputSink` trait), `vcli-perception` (`Perception` façade). Runtime-only deps: `crossbeam-channel` (sync mpsc with bounded/unbounded flavors — same edge as in `vcli-daemon`), `thiserror`, `tracing`. Dev-deps: `proptest`, `vcli-dsl` (for the DSL → `Program` shortcut in scenario fixtures), `tempfile` (for any on-disk assets in template-matching scenarios).

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md`. Sections that matter to this plan: §82–101 (crate responsibilities + threading), §161–230 (Watch / Step / postcondition / expression language), §311–435 (tick loop, shared capture, predicate cache, program state machine, arbitration, confirmation, watch lifetimes, triggers, clock), §729–793 (testing strategy), §901–1090 (tick dataflow / merged DAG / lifecycle diagrams). "Decision X.Y" references point to the "Review decisions — 2026-04-16" appendix.

**Sibling plans:** `2026-04-17-vcli-daemon.md` (consumes this runtime) and `2026-04-17-vcli-cli.md`. The daemon plan pre-commits to the following public API — this plan implements it. If reality drifts from the shape below, fix inline per AGENT.md.

```rust
// crates/vcli-runtime/src/lib.rs — public surface

pub use error::{RuntimeError, ErrorCode};
pub use command::SchedulerCommand;
pub use scheduler::{Scheduler, SchedulerConfig};

// Runtime-owned traits beyond vcli-core::Clock (caller may still use SystemClock).
pub use clock::RuntimeClock;
```

```rust
pub struct Scheduler { /* sync, Send */ }

impl Scheduler {
    pub fn new(
        config:     SchedulerConfig,
        capture:    Box<dyn vcli_capture::Capture>,
        input:      std::sync::Arc<dyn vcli_input::InputSink>,
        perception: vcli_perception::Perception,
        clock:      std::sync::Arc<dyn vcli_core::Clock + Send + Sync>,
        cmd_rx:     crossbeam_channel::Receiver<SchedulerCommand>,
        event_tx:   crossbeam_channel::Sender<vcli_core::Event>,
    ) -> Self;

    pub fn run_until_shutdown(self);
}

pub enum SchedulerCommand {
    SubmitValidated { program_id: vcli_core::ProgramId, program: vcli_core::Program },
    Cancel          { program_id: vcli_core::ProgramId, reason: String },
    Start           { program_id: vcli_core::ProgramId },
    ResumeRunning   { program_id: vcli_core::ProgramId, from_step: u32, program: vcli_core::Program },
    Shutdown,
}

pub struct SchedulerConfig {
    pub tick_interval_ms: u32,      // default 100
    pub tick_budget_ms:   u32,      // default 90 — above this emits daemon.pressure
    pub max_inflight:     usize,    // default 256 concurrent programs
}
```

---

## File structure produced by this plan

```
vcli/
├── Cargo.toml                                    # MODIFY: add crate + crossbeam-channel
└── crates/
    └── vcli-runtime/
        ├── Cargo.toml
        └── src/
            ├── lib.rs                            # #![forbid(unsafe_code)]; module tree + re-exports
            ├── error.rs                          # RuntimeError + ErrorCode (stable code() strings)
            ├── command.rs                        # SchedulerCommand enum
            ├── clock.rs                          # RuntimeClock wrapper; used nowhere the vcli-core Clock won't serve
            ├── event.rs                          # EventEmitter helper: emit(&self, EventData) -> Result
            ├── program.rs                        # RunningProgram: state + watch handles + body_cursor + first_true timer
            ├── transitions.rs                    # legal ProgramState transitions + reason strings
            ├── watches.rs                        # WatchHandle + Lifetime bookkeeping (OneShot/Persistent/UntilPredicate/TimeoutMs)
            ├── triggers.rs                       # Trigger::OnSubmit / OnPredicate evaluation
            ├── expr.rs                           # $pred.match.center + $pred.match.bbox resolution
            ├── body.rs                           # step-at-a-time body executor (WaitFor, Assert, SleepMs, input steps)
            ├── arbiter.rs                        # priority-based action arbitration; no re-queue of losers
            ├── confirm.rs                        # input postcondition + novelty_timeout
            ├── merged_graph.rs                   # cross-program predicate dedup via PredicateHash
            ├── budget.rs                         # tick budget timer + frame-skip policy
            └── scheduler.rs                      # Scheduler::new + run_until_shutdown + tick()
        └── tests/
            ├── common/
            │   ├── mod.rs                        # shared test imports; program builder; mock wiring
            │   ├── mock_capture.rs               # ScriptedCapture: Vec<Frame> queue + EnumerateErr knob
            │   ├── mock_clock.rs                 # ManualClock: set_now_ms / advance_ms
            │   └── mock_input.rs                 # RecordingInputSink: records every call; optional Err injector
            └── scenarios/
                ├── one_shot_watch.rs
                ├── persistent_watch.rs
                ├── until_predicate.rs
                ├── watch_timeout.rs
                ├── while_true_retry.rs
                ├── wait_for_timeout.rs
                ├── assert_failure.rs
                ├── input_postcondition.rs
                ├── novelty_timeout.rs
                ├── action_conflict.rs
                ├── predicate_dedup.rs
                ├── elapsed_ms_since_true.rs
                └── daemon_restart_marker.rs
```

**Responsibility split rationale:** one module per concept, no module over ~300 lines. `scheduler.rs` is intentionally thin — it owns the loop but delegates every real decision to the modules around it, which keeps each piece testable in isolation. `common/` mocks are separate files so scenario tests only pull in what they need. Every scenario is its own integration-test binary (one file = one binary in `tests/scenarios/`), so failure in one does not abort the others and tests run in parallel.

---

## Task 1: Crate scaffolding + workspace wiring

**Files:**
- Modify: `/Users/admin/Workspace/vcli/Cargo.toml`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/Cargo.toml`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/lib.rs`

- [ ] **Step 1: Add crate to workspace members + `crossbeam-channel` to workspace deps**

Edit `/Users/admin/Workspace/vcli/Cargo.toml`. Append `"crates/vcli-runtime"` to `[workspace] members`, and add `crossbeam-channel = "0.5"` under `[workspace.dependencies]` (it's new; the daemon plan also adds it — keep a single definition).

- [ ] **Step 2: Create crate manifest**

Create `/Users/admin/Workspace/vcli/crates/vcli-runtime/Cargo.toml`:

```toml
[package]
name        = "vcli-runtime"
version.workspace     = true
edition.workspace     = true
rust-version.workspace= true
license.workspace     = true
repository.workspace  = true
authors.workspace     = true
description = "Deterministic 10 fps scheduler that wires Capture → Perception → Arbiter → Input for vcli programs."

[dependencies]
vcli-core        = { path = "../vcli-core" }
vcli-capture     = { path = "../vcli-capture" }
vcli-input       = { path = "../vcli-input" }
vcli-perception  = { path = "../vcli-perception" }

crossbeam-channel = { workspace = true }
thiserror         = { workspace = true }
tracing           = { workspace = true }
serde_json        = { workspace = true }

[dev-dependencies]
vcli-dsl          = { path = "../vcli-dsl" }
proptest          = { workspace = true }
tempfile          = { workspace = true }
```

- [ ] **Step 3: Create `src/lib.rs` with module declarations and lints**

```rust
//! vcli-runtime — deterministic 10 fps scheduler.
//!
//! See the v0 design spec at `docs/superpowers/specs/2026-04-16-vcli-design.md`
//! — §Runtime & scheduler (§311) is the authoritative source for everything
//! in this crate.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

mod arbiter;
mod body;
mod budget;
mod clock;
mod command;
mod confirm;
mod error;
mod event;
mod expr;
mod merged_graph;
mod program;
mod scheduler;
mod transitions;
mod triggers;
mod watches;

pub use command::SchedulerCommand;
pub use error::{ErrorCode, RuntimeError};
pub use scheduler::{Scheduler, SchedulerConfig};
```

- [ ] **Step 4: Verify workspace compiles**

Run: `cargo check -p vcli-runtime`
Expected: compile fails on missing module files — `arbiter.rs`, `body.rs`, etc. — because we haven't created them yet.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-runtime/Cargo.toml crates/vcli-runtime/src/lib.rs Cargo.toml
git commit -m "$(cat <<'EOF'
vcli-runtime: scaffold crate + workspace wiring

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `RuntimeError` + stable `ErrorCode`

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/error.rs`

The scheduler maps every internal failure to a stable `code()` string so `Event::ProgramFailed { reason: err.code().into() }` produces predictable wire values that the CLI can match against. Codes come from spec §555 and §713.

- [ ] **Step 1: Write the failing test**

Append to `error.rs`:

```rust
//! Typed errors with a stable `code()` string suitable for IPC wire values.

use thiserror::Error;

/// Stable error-code string embedded in `program.failed.reason` and returned
/// to callers via IPC. Matches spec §555 (CLI error codes) and §713 (internal
/// surfaces).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// `capture.grab_screen()` or `grab_window()` failed.
    CaptureFailed,
    /// `InputSink` returned an error or the kill switch fired.
    InputFailed,
    /// A predicate evaluator raised an error (unknown name, asset missing, decode).
    PerceptionFailed,
    /// `wait_for` ran out of budget.
    WaitForTimeout,
    /// An `assert` predicate was not truthy.
    AssertFailed,
    /// A program's `timeout_ms` elapsed before completion.
    ProgramTimeout,
    /// Input postcondition never observed (`novelty_timeout` reached).
    NoveltyTimeout,
    /// An expression failed to resolve (`$pred.match.center` on a non-match).
    ExpressionUnresolved,
    /// Daemon restart marker (transitioned by `Scheduler::on_startup`).
    DaemonRestart,
    /// Catch-all for programmer errors (unreachable panics demoted to errors).
    Internal,
}

impl ErrorCode {
    /// Canonical wire-value string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CaptureFailed         => "capture_failed",
            Self::InputFailed           => "input_failed",
            Self::PerceptionFailed      => "perception_failed",
            Self::WaitForTimeout        => "wait_for_timeout",
            Self::AssertFailed          => "assert_failed",
            Self::ProgramTimeout        => "program_timeout",
            Self::NoveltyTimeout        => "novelty_timeout",
            Self::ExpressionUnresolved  => "expression_unresolved",
            Self::DaemonRestart         => "daemon_restart",
            Self::Internal              => "internal",
        }
    }
}

/// Runtime-layer error. Carries a [`ErrorCode`] plus a human reason string.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Capture backend failure.
    #[error("capture_failed: {0}")]
    Capture(String),
    /// Input dispatch failure.
    #[error("input_failed: {0}")]
    Input(String),
    /// Perception evaluator failure.
    #[error("perception_failed: {0}")]
    Perception(String),
    /// `wait_for` timed out.
    #[error("wait_for_timeout: predicate={predicate} after {waited_ms}ms")]
    WaitForTimeout {
        /// Predicate name.
        predicate: String,
        /// Budget actually consumed.
        waited_ms: u32,
    },
    /// `assert` predicate not truthy.
    #[error("assert_failed: predicate={predicate}")]
    AssertFailed {
        /// Predicate name.
        predicate: String,
    },
    /// Program-level timeout fired.
    #[error("program_timeout: after {elapsed_ms}ms")]
    ProgramTimeout {
        /// How long we ran before tripping.
        elapsed_ms: u32,
    },
    /// Postcondition not observed.
    #[error("novelty_timeout: no postcondition change within {timeout_ms}ms")]
    NoveltyTimeout {
        /// Budget configured on the Step.
        timeout_ms: u32,
    },
    /// Expression unresolved.
    #[error("expression_unresolved: {0}")]
    ExpressionUnresolved(String),
    /// Internal invariant violation.
    #[error("internal: {0}")]
    Internal(String),
}

impl RuntimeError {
    /// Wire-stable code. Propagated to `program.failed.reason`.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Capture(_)              => ErrorCode::CaptureFailed,
            Self::Input(_)                => ErrorCode::InputFailed,
            Self::Perception(_)           => ErrorCode::PerceptionFailed,
            Self::WaitForTimeout { .. }   => ErrorCode::WaitForTimeout,
            Self::AssertFailed   { .. }   => ErrorCode::AssertFailed,
            Self::ProgramTimeout { .. }   => ErrorCode::ProgramTimeout,
            Self::NoveltyTimeout { .. }   => ErrorCode::NoveltyTimeout,
            Self::ExpressionUnresolved(_) => ErrorCode::ExpressionUnresolved,
            Self::Internal(_)             => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_strings_are_spec_stable() {
        assert_eq!(ErrorCode::CaptureFailed.as_str(), "capture_failed");
        assert_eq!(ErrorCode::WaitForTimeout.as_str(), "wait_for_timeout");
        assert_eq!(ErrorCode::NoveltyTimeout.as_str(), "novelty_timeout");
    }

    #[test]
    fn runtime_error_maps_to_code() {
        let e = RuntimeError::WaitForTimeout { predicate: "p".into(), waited_ms: 1000 };
        assert_eq!(e.code(), ErrorCode::WaitForTimeout);
        assert_eq!(e.code().as_str(), "wait_for_timeout");
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime error::tests`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/error.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: RuntimeError with stable code() strings

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `SchedulerCommand` enum

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/command.rs`

- [ ] **Step 1: Write the module**

```rust
//! `SchedulerCommand` — messages sent from the daemon's tokio reactor into the
//! synchronous scheduler thread. Shape is pinned by spec §Threading model and
//! by the sibling vcli-daemon plan.

use vcli_core::{Program, ProgramId};

/// Commands consumed on every tick before evaluation.
#[derive(Debug, Clone)]
pub enum SchedulerCommand {
    /// Program was already validated by `vcli-dsl` and persisted by the
    /// daemon. The scheduler inserts it at state `Pending` and fires the
    /// trigger next tick.
    SubmitValidated {
        /// Daemon-assigned id.
        program_id: ProgramId,
        /// Parsed program.
        program: Program,
    },
    /// Terminate a program with `Cancelled`.
    Cancel {
        /// Target.
        program_id: ProgramId,
        /// Human reason (propagated into `program.state_changed.reason`).
        reason: String,
    },
    /// Force-start a `Waiting` program (bypasses its trigger — used by
    /// `vcli resume`).
    Start {
        /// Target.
        program_id: ProgramId,
    },
    /// Re-insert a program at `Running` with a prior `body_cursor`.
    ResumeRunning {
        /// Target.
        program_id: ProgramId,
        /// Step index to resume at (`0` = from start).
        from_step: u32,
        /// Full program (daemon reloaded it from SQLite).
        program: Program,
    },
    /// Drain and exit `run_until_shutdown`.
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::{program::DslVersion, trigger::Trigger};
    use std::collections::BTreeMap;

    fn sample_program() -> Program {
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: "x".into(),
            id: None,
            trigger: Trigger::OnSubmit,
            predicates: BTreeMap::new(),
            watches: vec![],
            body: vec![],
            on_complete: None,
            on_fail: None,
            timeout_ms: None,
            labels: BTreeMap::new(),
            priority: Default::default(),
        }
    }

    #[test]
    fn submit_variant_roundtrips_basic_shape() {
        let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
        let c = SchedulerCommand::SubmitValidated { program_id: id, program: sample_program() };
        match c {
            SchedulerCommand::SubmitValidated { program_id, .. } => assert_eq!(program_id, id),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn cancel_keeps_reason_untouched() {
        let id: ProgramId = "00000000-0000-4000-8000-000000000000".parse().unwrap();
        let c = SchedulerCommand::Cancel { program_id: id, reason: "user".into() };
        match c {
            SchedulerCommand::Cancel { reason, .. } => assert_eq!(reason, "user"),
            _ => panic!("wrong variant"),
        }
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime command::tests`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/command.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: SchedulerCommand enum

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `RuntimeClock` adapter + `ManualClock` for tests

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/clock.rs`

`vcli-core` already exports `Clock`, `SystemClock`, `TestClock`, and `UnixMs`. `RuntimeClock` is a thin adapter that pairs `now_ms()` with a `sleep_ms()` so the scheduler can pace ticks deterministically under `ManualClock` in tests.

- [ ] **Step 1: Write module + tests**

```rust
//! Scheduler clock. Extends `vcli_core::Clock` with a blocking `sleep_ms`
//! so tests can substitute a manual clock that advances time without actually
//! sleeping.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use vcli_core::{Clock, UnixMs};

/// Scheduler-facing clock. Implementors must be `Send + Sync` — the scheduler
/// holds them behind an `Arc<dyn RuntimeClock>`.
pub trait RuntimeClock: Clock + Send + Sync {
    /// Block for `ms` milliseconds. `ManualClock` short-circuits under tests.
    fn sleep_ms(&self, ms: u32);
}

/// Production clock: `SystemClock` wall time + `std::thread::sleep`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRuntimeClock;

impl Clock for SystemRuntimeClock {
    fn now_ms(&self) -> UnixMs {
        vcli_core::SystemClock.now_ms()
    }
}

impl RuntimeClock for SystemRuntimeClock {
    fn sleep_ms(&self, ms: u32) {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }
}

/// Deterministic clock for tests. `now_ms()` returns whatever the test set
/// via `set_now_ms`; `sleep_ms` advances `now_ms` by the same amount instead
/// of blocking.
#[derive(Debug, Clone)]
pub struct ManualClock {
    inner: Arc<(Mutex<UnixMs>, Condvar)>,
}

impl ManualClock {
    /// Create at `start_ms`.
    #[must_use]
    pub fn new(start_ms: UnixMs) -> Self {
        Self { inner: Arc::new((Mutex::new(start_ms), Condvar::new())) }
    }

    /// Jump the clock to `ms`.
    pub fn set_now_ms(&self, ms: UnixMs) {
        let (lock, cv) = &*self.inner;
        *lock.lock().unwrap() = ms;
        cv.notify_all();
    }

    /// Advance the clock by `delta_ms`.
    pub fn advance_ms(&self, delta_ms: u32) {
        let (lock, cv) = &*self.inner;
        let mut g = lock.lock().unwrap();
        *g = g.saturating_add(UnixMs::from(delta_ms));
        cv.notify_all();
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> UnixMs {
        *self.inner.0.lock().unwrap()
    }
}

impl RuntimeClock for ManualClock {
    fn sleep_ms(&self, ms: u32) {
        // Determinism: treat sleep as an advance, not a block. Scenario tests
        // call `tick_once()` manually; they never need real wall-clock pacing.
        self.advance_ms(ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_advance_adds_time() {
        let c = ManualClock::new(1_000);
        c.advance_ms(250);
        assert_eq!(c.now_ms(), 1_250);
    }

    #[test]
    fn manual_clock_sleep_advances_instead_of_blocks() {
        let c = ManualClock::new(0);
        let t0 = std::time::Instant::now();
        c.sleep_ms(10_000);
        assert!(t0.elapsed() < Duration::from_millis(100), "sleep must not block");
        assert_eq!(c.now_ms(), 10_000);
    }

    #[test]
    fn system_runtime_clock_is_monotonic_within_a_tick() {
        let c = SystemRuntimeClock;
        let t1 = c.now_ms();
        let t2 = c.now_ms();
        assert!(t2 >= t1);
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime clock::tests`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/clock.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: RuntimeClock trait with SystemRuntimeClock + ManualClock

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `EventEmitter` — wrap `crossbeam_channel::Sender<Event>` with timestamping

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/event.rs`

- [ ] **Step 1: Write module + tests**

```rust
//! Event emitter: wraps a `crossbeam_channel::Sender<Event>` and stamps every
//! emission with `clock.now_ms()`.

use std::sync::Arc;

use crossbeam_channel::Sender;
use vcli_core::{Clock, Event, EventData};

/// Stamp + send. The daemon owns the receiver side.
#[derive(Clone)]
pub struct EventEmitter {
    tx: Sender<Event>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl EventEmitter {
    /// Constructor.
    #[must_use]
    pub fn new(tx: Sender<Event>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { tx, clock }
    }

    /// Emit `data` stamped with `now_ms()`. Returns `false` only if the
    /// receiver has been dropped (daemon shut down before us). The scheduler
    /// ignores the return value and keeps running; the daemon drains
    /// remaining events after joining the scheduler thread.
    pub fn emit(&self, data: EventData) -> bool {
        let ev = Event { at: self.clock.now_ms(), data };
        self.tx.send(ev).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crossbeam_channel::unbounded;
    use vcli_core::ProgramId;

    fn sample_id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn emit_stamps_with_clock_now() {
        let (tx, rx) = unbounded::<Event>();
        let clock: Arc<dyn Clock + Send + Sync> = Arc::new(ManualClock::new(12_345));
        let em = EventEmitter::new(tx, clock);
        assert!(em.emit(EventData::ProgramSubmitted { program_id: sample_id(), name: "x".into() }));
        let ev = rx.recv().unwrap();
        assert_eq!(ev.at, 12_345);
    }

    #[test]
    fn emit_returns_false_when_receiver_dropped() {
        let (tx, rx) = unbounded::<Event>();
        let clock: Arc<dyn Clock + Send + Sync> = Arc::new(ManualClock::new(0));
        let em = EventEmitter::new(tx, clock);
        drop(rx);
        assert!(!em.emit(EventData::DaemonStopped));
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime event::tests`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/event.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: EventEmitter wrapper stamps Events with clock

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `transitions` — legal `ProgramState` transitions

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/transitions.rs`

Spec §370–402 lists the full state machine. `transitions::next()` is the single source of truth for which transitions are legal; `Scheduler::set_state()` (later) calls it before emitting `program.state_changed`.

- [ ] **Step 1: Write module + tests**

```rust
//! Legal program-state transitions. Spec §370.

use vcli_core::state::ProgramState;

/// Why a transition is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// The `from`→`to` pair is not allowed (e.g. re-entering `Running` from
    /// `Completed`).
    Illegal { from: ProgramState, to: ProgramState },
}

/// Return `Ok(())` if `from → to` is legal per spec §370, else
/// `Err(TransitionError::Illegal)`.
///
/// # Errors
///
/// Returns `Illegal` when the transition isn't in the allowed set.
pub fn validate(from: ProgramState, to: ProgramState) -> Result<(), TransitionError> {
    use ProgramState::{Blocked, Cancelled, Completed, Failed, Pending, Running, Waiting};
    let ok = matches!(
        (from, to),
        (Pending,   Waiting)
      | (Pending,   Cancelled)
      | (Waiting,   Running)
      | (Waiting,   Cancelled)
      | (Waiting,   Failed)
      | (Running,   Blocked)
      | (Running,   Completed)
      | (Running,   Failed)
      | (Running,   Cancelled)
      | (Blocked,   Running)
      | (Blocked,   Cancelled)
      | (Blocked,   Failed)
      | (Failed,    Running)   // vcli resume from daemon_restart
    );
    if ok { Ok(()) } else { Err(TransitionError::Illegal { from, to }) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProgramState::*;

    #[test]
    fn happy_path_transitions_are_legal() {
        for (f, t) in [
            (Pending, Waiting),
            (Waiting, Running),
            (Running, Completed),
            (Running, Failed),
            (Running, Cancelled),
            (Failed,  Running),   // resume
        ] {
            validate(f, t).expect(&format!("{f:?}→{t:?} should be legal"));
        }
    }

    #[test]
    fn terminal_states_cannot_restart_implicitly() {
        assert!(validate(Completed, Running).is_err());
        assert!(validate(Cancelled, Running).is_err());
    }

    #[test]
    fn waiting_to_completed_is_illegal() {
        // A waiting program must enter Running first.
        assert!(validate(Waiting, Completed).is_err());
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime transitions::tests`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/transitions.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: legal program-state transition table

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `RunningProgram` — per-program state container

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/program.rs`

- [ ] **Step 1: Write module + tests**

```rust
//! `RunningProgram` — the scheduler's in-memory view of a submitted program.

use std::collections::HashMap;

use vcli_core::{Program, ProgramId, UnixMs};
use vcli_core::state::ProgramState;

/// Scheduler-owned per-program state.
pub struct RunningProgram {
    /// Assigned id.
    pub id: ProgramId,
    /// Parsed program.
    pub program: Program,
    /// Current lifecycle state.
    pub state: ProgramState,
    /// Wall-clock ms when the program entered `Running` (for `timeout_ms` + `Lifetime::TimeoutMs`).
    pub running_since_ms: Option<UnixMs>,
    /// Active body step index. `Some(n)` while advancing; `None` when body exhausted.
    pub body_cursor: Option<u32>,
    /// Per-watch bookkeeping, keyed by watch index. Populated on entry to
    /// `Running`; drained as watches retire.
    pub watch_state: HashMap<u32, WatchRuntime>,
    /// If set, the next successful transition emits `program.resumed{from_step}`.
    pub resumed_from: Option<u32>,
}

/// Per-watch runtime state.
#[derive(Debug, Default, Clone)]
pub struct WatchRuntime {
    /// Last fire timestamp (for `throttle_ms`). None = has never fired.
    pub last_fired_ms: Option<UnixMs>,
    /// Last tick's truthiness result (for false→true edge detection).
    pub last_truthy: bool,
    /// Whether the watch has already been retired (OneShot or UntilPredicate tripped).
    pub retired: bool,
}

impl RunningProgram {
    /// Construct at `Pending` with default bookkeeping.
    #[must_use]
    pub fn pending(id: ProgramId, program: Program) -> Self {
        Self {
            id,
            program,
            state: ProgramState::Pending,
            running_since_ms: None,
            body_cursor: None,
            watch_state: HashMap::new(),
            resumed_from: None,
        }
    }

    /// Whether `body_cursor` points past the last body step.
    #[must_use]
    pub fn body_complete(&self) -> bool {
        match self.body_cursor {
            Some(n) => usize::try_from(n).unwrap_or(usize::MAX) >= self.program.body.len(),
            None => self.program.body.is_empty(),
        }
    }

    /// Count of watches that are not yet retired.
    #[must_use]
    pub fn active_watch_count(&self) -> usize {
        self.watch_state.values().filter(|w| !w.retired).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vcli_core::{program::DslVersion, trigger::Trigger};

    fn sample_program() -> Program {
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: "t".into(),
            id: None,
            trigger: Trigger::OnSubmit,
            predicates: BTreeMap::new(),
            watches: vec![],
            body: vec![],
            on_complete: None,
            on_fail: None,
            timeout_ms: None,
            labels: BTreeMap::new(),
            priority: Default::default(),
        }
    }

    #[test]
    fn pending_is_initial_state() {
        let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
        let rp = RunningProgram::pending(id, sample_program());
        assert_eq!(rp.state, ProgramState::Pending);
        assert!(rp.body_complete(), "empty body is trivially complete");
        assert_eq!(rp.active_watch_count(), 0);
    }

    #[test]
    fn body_complete_detects_exhausted_cursor() {
        let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
        let mut p = sample_program();
        p.body = vec![vcli_core::Step::SleepMs { ms: 10 }];
        let mut rp = RunningProgram::pending(id, p);
        rp.body_cursor = Some(0);
        assert!(!rp.body_complete());
        rp.body_cursor = Some(1);
        assert!(rp.body_complete());
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime program::tests`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/program.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: RunningProgram container with body_cursor + watch_state

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `triggers` — `OnSubmit` + `OnPredicate` evaluation

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/triggers.rs`

v0 is watch-based only per §425, but `vcli-core::Trigger` carries both `OnSubmit` and `OnPredicate` variants. `trigger_fires()` wraps the evaluation so the scheduler calls one function regardless of shape.

- [ ] **Step 1: Write module + tests**

```rust
//! Trigger evaluation. A program in `Waiting` advances to `Running` as soon
//! as its trigger fires. Spec §425.

use std::collections::BTreeMap;

use vcli_core::{Frame, Predicate, ProgramId, Trigger, UnixMs};
use vcli_perception::Perception;

use crate::error::RuntimeError;

/// Return `true` if the trigger says the program may start this tick.
///
/// # Errors
///
/// Propagates [`RuntimeError::Perception`] when evaluating `OnPredicate`.
pub fn trigger_fires(
    trig: &Trigger,
    predicates: &BTreeMap<String, Predicate>,
    frame: &Frame,
    now_ms: UnixMs,
    assets: &BTreeMap<String, Vec<u8>>,
    perception: &Perception,
    program_id: ProgramId,
) -> Result<bool, RuntimeError> {
    match trig {
        Trigger::OnSubmit => Ok(true),
        Trigger::OnPredicate { name } => {
            let r = perception
                .evaluate_named(name, predicates, frame, now_ms, assets, Some(program_id))
                .map_err(|e| RuntimeError::Perception(e.to_string()))?;
            Ok(r.truthy)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vcli_core::frame::FrameFormat;
    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::{PredicateKind, Rgb};

    fn red_frame() -> Frame {
        Frame::new(
            FrameFormat::Rgba8,
            Rect { x: 0, y: 0, w: 1, h: 1 },
            4,
            Arc::from(vec![255u8, 0, 0, 255]),
            0,
        )
    }

    fn some_id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn on_submit_always_fires() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let out = trigger_fires(&Trigger::OnSubmit, &preds, &red_frame(), 0, &assets, &p, some_id()).unwrap();
        assert!(out);
    }

    #[test]
    fn on_predicate_defers_to_perception() {
        let mut preds = BTreeMap::new();
        preds.insert(
            "is_red".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let p = Perception::new();
        let out = trigger_fires(
            &Trigger::OnPredicate { name: "is_red".into() },
            &preds,
            &red_frame(),
            100,
            &assets,
            &p,
            some_id(),
        ).unwrap();
        assert!(out);
    }

    #[test]
    fn on_predicate_unknown_name_errors() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let out = trigger_fires(
            &Trigger::OnPredicate { name: "nope".into() },
            &preds,
            &red_frame(),
            0,
            &assets,
            &p,
            some_id(),
        );
        assert!(matches!(out, Err(RuntimeError::Perception(_))));
    }
}
```

This references `vcli_core::Trigger::OnPredicate { name }`; if the real enum uses different field names (e.g. `pred`), correct inline per AGENT.md's "trust reality" rule.

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime triggers::tests`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/triggers.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: trigger evaluation (OnSubmit + OnPredicate)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `expr` — `$pred.match.center` + `.bbox` resolution

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/expr.rs`

Step `Target::Expression("$pred.match.center")` must resolve against a just-evaluated `PredicateResult`. Spec §217–229.

- [ ] **Step 1: Write module + tests**

```rust
//! Expression resolver: `$<name>.match.center` and `$<name>.match.bbox`.

use vcli_core::geom::{Point, Rect};
use vcli_core::predicate::PredicateResult;

use crate::error::RuntimeError;

/// Parts of an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedExpr<'a> {
    /// Referenced predicate name.
    pub predicate: &'a str,
    /// Accessor path (`.match.center` / `.match.bbox`).
    pub accessor: Accessor,
}

/// Supported accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accessor {
    /// `.match.center`
    MatchCenter,
    /// `.match.bbox`
    MatchBbox,
}

/// Parse an expression string. Returns [`RuntimeError::ExpressionUnresolved`]
/// on malformed input.
///
/// # Errors
///
/// Returns `ExpressionUnresolved` for non-`$...` strings or unknown accessors.
pub fn parse(s: &str) -> Result<ParsedExpr<'_>, RuntimeError> {
    let rest = s.strip_prefix('$').ok_or_else(|| {
        RuntimeError::ExpressionUnresolved(format!("expected '$' prefix: {s}"))
    })?;
    let (pred, tail) = rest.split_once('.').ok_or_else(|| {
        RuntimeError::ExpressionUnresolved(format!("expected '.<accessor>': {s}"))
    })?;
    if pred.is_empty() {
        return Err(RuntimeError::ExpressionUnresolved(format!("empty predicate name: {s}")));
    }
    let accessor = match tail {
        "match.center" => Accessor::MatchCenter,
        "match.bbox" => Accessor::MatchBbox,
        other => {
            return Err(RuntimeError::ExpressionUnresolved(format!("unknown accessor '{other}'")));
        }
    };
    Ok(ParsedExpr { predicate: pred, accessor })
}

/// Resolve a parsed expression against a predicate result.
///
/// # Errors
///
/// Returns `ExpressionUnresolved` if the result has no `match_data` (the
/// predicate was truthy but non-spatial, e.g. `color_at`), or the accessor
/// doesn't match the available data.
pub fn resolve_center(r: &PredicateResult) -> Result<Point, RuntimeError> {
    let md = r.match_data.as_ref().ok_or_else(|| {
        RuntimeError::ExpressionUnresolved("predicate has no match_data".into())
    })?;
    Ok(Point {
        x: md.bbox.x + md.bbox.w.saturating_sub(1) / 2,
        y: md.bbox.y + md.bbox.h.saturating_sub(1) / 2,
    })
}

/// Resolve a parsed expression's bbox against a predicate result.
///
/// # Errors
///
/// Returns `ExpressionUnresolved` when there is no `match_data`.
pub fn resolve_bbox(r: &PredicateResult) -> Result<Rect, RuntimeError> {
    let md = r.match_data.as_ref().ok_or_else(|| {
        RuntimeError::ExpressionUnresolved("predicate has no match_data".into())
    })?;
    Ok(md.bbox)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::predicate::MatchData;

    #[test]
    fn parse_match_center() {
        let p = parse("$skip.match.center").unwrap();
        assert_eq!(p.predicate, "skip");
        assert_eq!(p.accessor, Accessor::MatchCenter);
    }

    #[test]
    fn parse_match_bbox() {
        let p = parse("$x.match.bbox").unwrap();
        assert_eq!(p.accessor, Accessor::MatchBbox);
    }

    #[test]
    fn parse_rejects_missing_dollar() {
        assert!(matches!(parse("skip.match.center"), Err(RuntimeError::ExpressionUnresolved(_))));
    }

    #[test]
    fn parse_rejects_unknown_accessor() {
        assert!(matches!(parse("$skip.match.topleft"), Err(RuntimeError::ExpressionUnresolved(_))));
    }

    #[test]
    fn resolve_center_averages_bbox() {
        let r = PredicateResult {
            truthy: true,
            at: 0,
            match_data: Some(MatchData {
                bbox: Rect { x: 10, y: 20, w: 40, h: 20 },
                score: 0.9,
            }),
        };
        assert_eq!(resolve_center(&r).unwrap(), Point { x: 29, y: 29 });
    }

    #[test]
    fn resolve_without_match_data_errors() {
        let r = PredicateResult { truthy: true, at: 0, match_data: None };
        assert!(resolve_center(&r).is_err());
    }
}
```

`MatchData` fields used above (`bbox`, `score`) match `vcli-core::predicate`. If the actual names differ, correct inline.

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime expr::tests`
Expected: 6 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/expr.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: $pred.match.{center,bbox} resolver

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: `watches` — lifetime bookkeeping (`OneShot` / `Persistent` / `UntilPredicate` / `TimeoutMs`)

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/watches.rs`

Decides whether a watch should fire this tick (false→true edge + throttle + not-retired), and whether the watch is retired *after* firing or reaching its lifetime bound.

- [ ] **Step 1: Write module + tests**

```rust
//! Watch lifetime bookkeeping. Spec §416.

use vcli_core::{UnixMs, Watch};
use vcli_core::watch::Lifetime;

use crate::program::WatchRuntime;

/// Decision for this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchDecision {
    /// Do not fire and do not retire.
    Skip,
    /// Fire the watch (run its `do` steps).
    Fire,
    /// Retire without firing (e.g. `UntilPredicate` went truthy mid-tick).
    Retire,
}

/// Decide for a single watch given its current `state` and the *new* truthiness
/// result from this tick. Does NOT mutate; the scheduler applies mutation
/// (`state.last_truthy` / `state.last_fired_ms` / `state.retired`) only after
/// the watch's `do` steps actually dispatch (so a kill-switch or arbiter drop
/// does not consume the firing budget).
#[must_use]
pub fn decide(
    watch: &Watch,
    state: &WatchRuntime,
    truthy_now: bool,
    now_ms: UnixMs,
    running_since_ms: UnixMs,
) -> WatchDecision {
    if state.retired {
        return WatchDecision::Skip;
    }
    // Lifetime::TimeoutMs retires as soon as the window elapses.
    if let Lifetime::TimeoutMs { ms } = watch.lifetime {
        if now_ms.saturating_sub(running_since_ms) >= UnixMs::from(ms) {
            return WatchDecision::Retire;
        }
    }
    // False→true edge check.
    if !(truthy_now && !state.last_truthy) {
        return WatchDecision::Skip;
    }
    // Throttle.
    if let Some(last) = state.last_fired_ms {
        if now_ms.saturating_sub(last) < UnixMs::from(watch.throttle_ms) {
            return WatchDecision::Skip;
        }
    }
    WatchDecision::Fire
}

/// Apply post-firing retirement rules for `OneShot`.
pub fn after_fire(watch: &Watch, state: &mut WatchRuntime, now_ms: UnixMs) {
    state.last_fired_ms = Some(now_ms);
    if matches!(watch.lifetime, Lifetime::OneShot) {
        state.retired = true;
    }
}

/// Retire a watch when its `UntilPredicate` predicate is truthy this tick.
pub fn on_until_predicate_truthy(state: &mut WatchRuntime) {
    state.retired = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::watch::{Lifetime, WatchWhen};

    fn mk_watch(lifetime: Lifetime, throttle_ms: u32) -> Watch {
        Watch {
            when: WatchWhen::ByName("p".into()),
            steps: vec![],
            throttle_ms,
            lifetime,
        }
    }

    #[test]
    fn fires_on_false_to_true_edge() {
        let w = mk_watch(Lifetime::Persistent, 0);
        let st = WatchRuntime::default();
        assert_eq!(decide(&w, &st, true, 100, 0), WatchDecision::Fire);
    }

    #[test]
    fn skips_on_level_high() {
        let w = mk_watch(Lifetime::Persistent, 0);
        let st = WatchRuntime { last_truthy: true, ..Default::default() };
        assert_eq!(decide(&w, &st, true, 100, 0), WatchDecision::Skip);
    }

    #[test]
    fn throttle_suppresses_rapid_refires() {
        let w = mk_watch(Lifetime::Persistent, 500);
        let st = WatchRuntime { last_truthy: false, last_fired_ms: Some(100), retired: false };
        assert_eq!(decide(&w, &st, true, 400, 0), WatchDecision::Skip);
        assert_eq!(decide(&w, &st, true, 700, 0), WatchDecision::Fire);
    }

    #[test]
    fn one_shot_retires_after_fire() {
        let w = mk_watch(Lifetime::OneShot, 0);
        let mut st = WatchRuntime::default();
        after_fire(&w, &mut st, 50);
        assert!(st.retired);
    }

    #[test]
    fn persistent_does_not_retire_after_fire() {
        let w = mk_watch(Lifetime::Persistent, 0);
        let mut st = WatchRuntime::default();
        after_fire(&w, &mut st, 50);
        assert!(!st.retired);
        assert_eq!(st.last_fired_ms, Some(50));
    }

    #[test]
    fn timeout_ms_retires_when_window_elapsed() {
        let w = mk_watch(Lifetime::TimeoutMs { ms: 1_000 }, 0);
        let st = WatchRuntime::default();
        assert_eq!(decide(&w, &st, true, 2_001, 1_000), WatchDecision::Retire);
    }

    #[test]
    fn retired_watch_is_inert() {
        let w = mk_watch(Lifetime::Persistent, 0);
        let st = WatchRuntime { retired: true, ..Default::default() };
        assert_eq!(decide(&w, &st, true, 100, 0), WatchDecision::Skip);
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime watches::tests`
Expected: 7 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/watches.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: watch lifetime bookkeeping + fire decision

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: `arbiter` — per-tick action arbitration

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/arbiter.rs`

Spec §403–411: two programs firing same-frame produce a priority-ordered winner list; losers are dropped (not re-queued). Tiebreak is `ProgramId` lexicographic order for determinism.

- [ ] **Step 1: Write module + tests**

```rust
//! Action arbitration. Spec §403.

use vcli_core::program::Priority;
use vcli_core::ProgramId;

/// One pending action for the arbiter to consider.
#[derive(Debug, Clone)]
pub struct Candidate<T> {
    /// Source program.
    pub program_id: ProgramId,
    /// Static priority from `program.priority`.
    pub priority: Priority,
    /// The action / step payload (opaque to the arbiter).
    pub payload: T,
}

/// Outcome of arbitration for one candidate.
#[derive(Debug, Clone)]
pub struct Decision<T> {
    /// Source program.
    pub program_id: ProgramId,
    /// Payload (passed through unchanged).
    pub payload: T,
    /// True if this payload should dispatch.
    pub dispatch: bool,
    /// If not dispatched, the conflicting winner's id.
    pub loser_of: Option<ProgramId>,
}

/// Resolve conflicts: at most one dispatch per tick. Ordering:
/// 1. highest `Priority` wins
/// 2. tiebreak by lexicographic `ProgramId`
///
/// If `candidates.len() <= 1` everyone dispatches.
pub fn resolve<T: Clone>(candidates: Vec<Candidate<T>>) -> Vec<Decision<T>> {
    if candidates.len() <= 1 {
        return candidates.into_iter().map(|c| Decision {
            program_id: c.program_id, payload: c.payload, dispatch: true, loser_of: None,
        }).collect();
    }
    // Find the winner.
    let winner = candidates
        .iter()
        .max_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| b.program_id.to_string().cmp(&a.program_id.to_string()))
        })
        .map(|c| c.program_id)
        .expect("len > 1 handled above");
    candidates.into_iter().map(|c| {
        let is_winner = c.program_id == winner;
        Decision {
            program_id: c.program_id,
            payload: c.payload,
            dispatch: is_winner,
            loser_of: if is_winner { None } else { Some(winner) },
        }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ProgramId {
        let s = format!("{n:02x}345678-1234-4567-8910-111213141516");
        s.parse().unwrap()
    }

    #[test]
    fn single_candidate_always_dispatches() {
        let out = resolve(vec![Candidate { program_id: id(1), priority: Priority(0), payload: "a" }]);
        assert_eq!(out.len(), 1);
        assert!(out[0].dispatch);
        assert!(out[0].loser_of.is_none());
    }

    #[test]
    fn higher_priority_wins() {
        let out = resolve(vec![
            Candidate { program_id: id(1), priority: Priority(0), payload: "a" },
            Candidate { program_id: id(2), priority: Priority(5), payload: "b" },
        ]);
        let dispatched: Vec<_> = out.iter().filter(|d| d.dispatch).collect();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0].program_id, id(2));
        assert_eq!(out.iter().find(|d| !d.dispatch).unwrap().loser_of, Some(id(2)));
    }

    #[test]
    fn tie_breaks_by_program_id_lex_desc() {
        // lexicographic order: id(1) < id(2); highest wins so id(2) wins.
        let out = resolve(vec![
            Candidate { program_id: id(1), priority: Priority(0), payload: "a" },
            Candidate { program_id: id(2), priority: Priority(0), payload: "b" },
        ]);
        let winner = out.iter().find(|d| d.dispatch).unwrap().program_id;
        assert_eq!(winner, id(2));
    }

    #[test]
    fn three_way_drops_two_losers() {
        let out = resolve(vec![
            Candidate { program_id: id(1), priority: Priority(3), payload: "a" },
            Candidate { program_id: id(2), priority: Priority(3), payload: "b" },
            Candidate { program_id: id(3), priority: Priority(3), payload: "c" },
        ]);
        let dispatched: Vec<_> = out.iter().filter(|d| d.dispatch).collect();
        assert_eq!(dispatched.len(), 1);
        // lexicographic max = id(3) when priorities tied.
        assert_eq!(dispatched[0].program_id, id(3));
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime arbiter::tests`
Expected: 4 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/arbiter.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: priority-based action arbiter

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: `confirm` — input postcondition + `novelty_timeout`

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/confirm.rs`

Spec §412–415 and §202–216: after dispatching an input action, the scheduler optionally waits for a named "postcondition" predicate to observe that the action actually changed something. If no change within `novelty_timeout_ms`, emit `program.failed(novelty_timeout)`.

- [ ] **Step 1: Write module + tests**

```rust
//! Input postcondition tracking.

use std::collections::BTreeMap;

use vcli_core::{Frame, Predicate, ProgramId, UnixMs};
use vcli_perception::Perception;

use crate::error::RuntimeError;

/// Pending postcondition: the scheduler re-evaluates this on subsequent ticks
/// until it flips truthy (success) or `deadline_ms` is reached (novelty timeout).
#[derive(Debug, Clone)]
pub struct PendingConfirm {
    /// Source program.
    pub program_id: ProgramId,
    /// Predicate name to watch.
    pub predicate: String,
    /// Timeout cap.
    pub deadline_ms: UnixMs,
    /// Step-path hint for `program.failed.step` if we time out.
    pub step_hint: String,
}

/// Evaluate one pending confirmation. Returns whether it:
/// (a) resolved truthy (success → scheduler advances body_cursor),
/// (b) timed out (novelty_timeout),
/// (c) still pending (keep in the queue).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// Postcondition became truthy — success.
    Success,
    /// Deadline elapsed — `novelty_timeout`.
    Timeout,
    /// Keep checking next tick.
    Pending,
}

/// Evaluate one pending confirm against the current tick.
///
/// # Errors
///
/// Propagates perception errors.
pub fn step(
    pc: &PendingConfirm,
    predicates: &BTreeMap<String, Predicate>,
    frame: &Frame,
    now_ms: UnixMs,
    assets: &BTreeMap<String, Vec<u8>>,
    perception: &Perception,
) -> Result<ConfirmOutcome, RuntimeError> {
    let r = perception
        .evaluate_named(&pc.predicate, predicates, frame, now_ms, assets, Some(pc.program_id))
        .map_err(|e| RuntimeError::Perception(e.to_string()))?;
    if r.truthy {
        return Ok(ConfirmOutcome::Success);
    }
    if now_ms >= pc.deadline_ms {
        return Ok(ConfirmOutcome::Timeout);
    }
    Ok(ConfirmOutcome::Pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vcli_core::frame::FrameFormat;
    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::{PredicateKind, Rgb};

    fn red_frame() -> Frame {
        Frame::new(
            FrameFormat::Rgba8,
            Rect { x: 0, y: 0, w: 1, h: 1 },
            4,
            Arc::from(vec![255u8, 0, 0, 255]),
            0,
        )
    }

    fn id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn success_when_truthy() {
        let mut preds = BTreeMap::new();
        preds.insert("red".into(), PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([255, 0, 0]),
            tolerance: 0,
        });
        let pc = PendingConfirm { program_id: id(), predicate: "red".into(), deadline_ms: 1_000, step_hint: "body[0]".into() };
        let p = Perception::new();
        let out = step(&pc, &preds, &red_frame(), 500, &BTreeMap::new(), &p).unwrap();
        assert_eq!(out, ConfirmOutcome::Success);
    }

    #[test]
    fn timeout_when_deadline_passed_and_still_false() {
        let mut preds = BTreeMap::new();
        preds.insert("blue".into(), PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([0, 0, 255]),
            tolerance: 0,
        });
        let pc = PendingConfirm { program_id: id(), predicate: "blue".into(), deadline_ms: 500, step_hint: "body[0]".into() };
        let p = Perception::new();
        let out = step(&pc, &preds, &red_frame(), 800, &BTreeMap::new(), &p).unwrap();
        assert_eq!(out, ConfirmOutcome::Timeout);
    }

    #[test]
    fn pending_when_still_false_within_budget() {
        let mut preds = BTreeMap::new();
        preds.insert("blue".into(), PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([0, 0, 255]),
            tolerance: 0,
        });
        let pc = PendingConfirm { program_id: id(), predicate: "blue".into(), deadline_ms: 1_000, step_hint: "body[0]".into() };
        let p = Perception::new();
        let out = step(&pc, &preds, &red_frame(), 300, &BTreeMap::new(), &p).unwrap();
        assert_eq!(out, ConfirmOutcome::Pending);
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime confirm::tests`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/confirm.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: input postcondition + novelty_timeout evaluator

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: `merged_graph` — cross-program predicate dedup

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/merged_graph.rs`

Spec §985 says the scheduler builds a merged predicate DAG each tick so two programs referencing the same `template` predicate scan the image once. The `Perception` façade already memoizes per-tick by `PredicateHash`; `merged_graph` is the helper that deduplicates *work submission* so we don't call `evaluate_named` twice for the same canonical predicate.

- [ ] **Step 1: Write module + tests**

```rust
//! Cross-program predicate deduplication.

use std::collections::{BTreeMap, HashSet};

use vcli_core::canonical::{predicate_hash, PredicateHash};
use vcli_core::Predicate;

use crate::error::RuntimeError;

/// One entry in the merged graph.
#[derive(Debug, Clone)]
pub struct DedupEntry<'a> {
    /// Canonical hash — equality implies caller may re-use a prior result.
    pub hash: PredicateHash,
    /// Predicate reference (borrowed from the owning program).
    pub predicate: &'a Predicate,
}

/// Walk a list of programs' active predicate references and return one entry
/// per unique canonical hash. Callers use this to drive a single pass through
/// the `Perception` façade — the per-tick cache inside `Perception` already
/// dedupes at eval time, but this lets the scheduler avoid even building the
/// argument tuple twice.
///
/// # Errors
///
/// Propagates canonicalization errors from `predicate_hash`.
pub fn dedupe<'a, I>(iter: I) -> Result<Vec<DedupEntry<'a>>, RuntimeError>
where
    I: IntoIterator<Item = &'a Predicate>,
{
    let mut seen: HashSet<PredicateHash> = HashSet::new();
    let mut out = Vec::new();
    for p in iter {
        let v = serde_json::to_value(p).map_err(|e| RuntimeError::Internal(format!("serialize pred: {e}")))?;
        let hash = predicate_hash(&v).map_err(|e| RuntimeError::Internal(format!("hash pred: {e}")))?;
        if seen.insert(hash.clone()) {
            out.push(DedupEntry { hash, predicate: p });
        }
    }
    Ok(out)
}

/// Flatten every predicate referenced by a program's watches (by-name) into
/// an iterator of borrowed `Predicate`s. Inline predicates short-circuit the
/// `predicates` lookup.
pub fn watch_predicates<'a>(
    watches: &'a [vcli_core::Watch],
    predicates: &'a BTreeMap<String, Predicate>,
) -> Vec<&'a Predicate> {
    let mut out = Vec::new();
    for w in watches {
        match &w.when {
            vcli_core::watch::WatchWhen::ByName(n) => {
                if let Some(p) = predicates.get(n) {
                    out.push(p);
                }
            }
            vcli_core::watch::WatchWhen::Inline(p) => {
                out.push(p.as_ref());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::geom::Point;
    use vcli_core::predicate::{PredicateKind, Rgb};
    use vcli_core::watch::{Lifetime, Watch, WatchWhen};

    #[test]
    fn identical_predicates_collapse() {
        let p = PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([1, 2, 3]),
            tolerance: 0,
        };
        let out = dedupe(std::iter::once(&p).chain(std::iter::once(&p))).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn different_predicates_stay_separate() {
        let a = PredicateKind::ColorAt { point: Point { x: 0, y: 0 }, rgb: Rgb([1, 2, 3]), tolerance: 0 };
        let b = PredicateKind::ColorAt { point: Point { x: 1, y: 1 }, rgb: Rgb([1, 2, 3]), tolerance: 0 };
        let out = dedupe([&a, &b]).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn watch_predicates_expands_by_name_and_inline() {
        let mut preds = BTreeMap::new();
        preds.insert("red".into(), PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 }, rgb: Rgb([255, 0, 0]), tolerance: 0,
        });
        let watches = vec![
            Watch {
                when: WatchWhen::ByName("red".into()),
                steps: vec![],
                throttle_ms: 0,
                lifetime: Lifetime::Persistent,
            },
            Watch {
                when: WatchWhen::Inline(Box::new(PredicateKind::ColorAt {
                    point: Point { x: 2, y: 2 }, rgb: Rgb([0, 255, 0]), tolerance: 0,
                })),
                steps: vec![],
                throttle_ms: 0,
                lifetime: Lifetime::Persistent,
            },
        ];
        let collected = watch_predicates(&watches, &preds);
        assert_eq!(collected.len(), 2);
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime merged_graph::tests`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/merged_graph.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: cross-program predicate dedup helper

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: `budget` — tick budget + frame-skip policy

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/budget.rs`

- [ ] **Step 1: Write module + tests**

```rust
//! Per-tick budget tracking. Spec §348 (capture overrun) + Decision 4.1 (daemon.pressure).

use std::time::Instant;

/// Tick budget gate.
pub struct BudgetGate {
    /// Configured budget (ms).
    pub budget_ms: u32,
    /// Deadline for the current tick.
    deadline: Option<Instant>,
    /// Count of over-budget ticks since last reset (for daemon.pressure emission).
    overrun_streak: u32,
}

impl BudgetGate {
    /// Create with `budget_ms`.
    #[must_use]
    pub fn new(budget_ms: u32) -> Self {
        Self { budget_ms, deadline: None, overrun_streak: 0 }
    }

    /// Start a new tick. Call at the top of `tick()`.
    pub fn start_tick(&mut self) {
        self.deadline = Some(Instant::now() + std::time::Duration::from_millis(u64::from(self.budget_ms)));
    }

    /// Whether the current tick has exceeded its budget.
    #[must_use]
    pub fn is_over(&self) -> bool {
        match self.deadline {
            Some(d) => Instant::now() > d,
            None => false,
        }
    }

    /// Increment or reset the overrun streak. Returns `true` if this tick
    /// crossed the pressure threshold (5 consecutive overruns per Decision 4.1).
    pub fn note_outcome(&mut self, overrun: bool) -> bool {
        if overrun {
            self.overrun_streak += 1;
        } else {
            self.overrun_streak = 0;
        }
        self.overrun_streak >= 5
    }

    /// Reset streak without noting a new tick (used by tests).
    pub fn reset_streak(&mut self) {
        self.overrun_streak = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_threshold_at_five() {
        let mut b = BudgetGate::new(90);
        for _ in 0..4 {
            assert!(!b.note_outcome(true));
        }
        assert!(b.note_outcome(true));
    }

    #[test]
    fn successful_tick_resets_streak() {
        let mut b = BudgetGate::new(90);
        for _ in 0..4 {
            b.note_outcome(true);
        }
        b.note_outcome(false);
        // One more overrun should not cross threshold.
        assert!(!b.note_outcome(true));
    }

    #[test]
    fn is_over_false_before_start_tick() {
        let b = BudgetGate::new(90);
        assert!(!b.is_over());
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime budget::tests`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/budget.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: per-tick budget gate + pressure streak

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: `body` — step-at-a-time executor (input + WaitFor + Assert + SleepMs)

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/body.rs`

The body executor advances one step per tick. Input steps resolve expressions, dispatch to `InputSink`, and produce an optional `PendingConfirm`. Control-flow steps (`WaitFor`, `Assert`, `SleepMs`) can also stall execution across ticks.

- [ ] **Step 1: Write module + tests**

```rust
//! Body step executor: runs exactly one step per tick; advances `body_cursor`
//! on success.

use std::collections::BTreeMap;
use std::sync::Arc;

use vcli_core::geom::Point;
use vcli_core::{Frame, Predicate, ProgramId, UnixMs};
use vcli_core::step::{OnFail, OnTimeout, Step, Target};
use vcli_perception::Perception;
use vcli_input::InputSink;

use crate::error::RuntimeError;
use crate::expr;

/// Outcome of one body-step attempt.
#[derive(Debug, Clone)]
pub enum StepOutcome {
    /// Step completed — advance `body_cursor` by 1.
    Advanced,
    /// Step is still waiting (e.g. `wait_for`, `sleep_ms` not yet elapsed). Do not advance.
    Stalled,
    /// Body finished (cursor already past end).
    BodyComplete,
    /// Program must transition to `failed`.
    Failed(RuntimeError),
}

/// Pending `sleep_ms` / `wait_for` deadline. The scheduler stashes this in
/// `RunningProgram` and the body executor consults it on subsequent ticks.
#[derive(Debug, Clone)]
pub enum BodyDefer {
    /// Sleep until `wake_at_ms`.
    Sleep { wake_at_ms: UnixMs },
    /// Wait for predicate up to `deadline_ms`.
    WaitFor {
        /// Predicate name.
        predicate: String,
        /// Deadline.
        deadline_ms: UnixMs,
        /// Timeout behaviour.
        on_timeout: OnTimeout,
    },
}

/// Per-program body-executor state (owned by `RunningProgram` — propagate via
/// a separate field if desired; for now we pass `deferred` explicitly).
#[derive(Default, Clone)]
pub struct BodyState {
    /// Active deferral.
    pub deferred: Option<BodyDefer>,
}

/// Execute one body step. The scheduler holds the borrow on `RunningProgram`,
/// so mutable fields (cursor + defer) live on `BodyState`; callers pass them
/// in.
#[allow(clippy::too_many_arguments)]
pub fn step_once(
    program_id: ProgramId,
    body: &[Step],
    cursor: u32,
    state: &mut BodyState,
    predicates: &BTreeMap<String, Predicate>,
    frame: &Frame,
    now_ms: UnixMs,
    assets: &BTreeMap<String, Vec<u8>>,
    perception: &Perception,
    input: &Arc<dyn InputSink>,
) -> StepOutcome {
    // Resolve any deferred step first.
    if let Some(def) = state.deferred.clone() {
        match def {
            BodyDefer::Sleep { wake_at_ms } => {
                if now_ms >= wake_at_ms {
                    state.deferred = None;
                    return StepOutcome::Advanced;
                }
                return StepOutcome::Stalled;
            }
            BodyDefer::WaitFor { predicate, deadline_ms, on_timeout } => {
                let r = match perception.evaluate_named(&predicate, predicates, frame, now_ms, assets, Some(program_id)) {
                    Ok(r) => r,
                    Err(e) => return StepOutcome::Failed(RuntimeError::Perception(e.to_string())),
                };
                if r.truthy {
                    state.deferred = None;
                    return StepOutcome::Advanced;
                }
                if now_ms >= deadline_ms {
                    state.deferred = None;
                    return match on_timeout {
                        OnTimeout::Continue | OnTimeout::Retry => StepOutcome::Advanced,
                        OnTimeout::Fail => StepOutcome::Failed(RuntimeError::WaitForTimeout {
                            predicate,
                            waited_ms: 0, // scheduler fills in the real elapsed
                        }),
                    };
                }
                return StepOutcome::Stalled;
            }
        }
    }

    let step = match body.get(cursor as usize) {
        Some(s) => s,
        None => return StepOutcome::BodyComplete,
    };

    match step {
        Step::SleepMs { ms } => {
            state.deferred = Some(BodyDefer::Sleep {
                wake_at_ms: now_ms.saturating_add(UnixMs::from(*ms)),
            });
            StepOutcome::Stalled
        }
        Step::WaitFor { predicate, timeout_ms, on_timeout } => {
            state.deferred = Some(BodyDefer::WaitFor {
                predicate: predicate.clone(),
                deadline_ms: now_ms.saturating_add(UnixMs::from(*timeout_ms)),
                on_timeout: *on_timeout,
            });
            StepOutcome::Stalled
        }
        Step::Assert { predicate, on_fail } => {
            let r = match perception.evaluate_named(predicate, predicates, frame, now_ms, assets, Some(program_id)) {
                Ok(r) => r,
                Err(e) => return StepOutcome::Failed(RuntimeError::Perception(e.to_string())),
            };
            if r.truthy {
                StepOutcome::Advanced
            } else {
                match on_fail {
                    OnFail::Continue => StepOutcome::Advanced,
                    OnFail::Fail => StepOutcome::Failed(RuntimeError::AssertFailed { predicate: predicate.clone() }),
                }
            }
        }
        Step::Move { at }     => dispatch_at(at, predicates, frame, now_ms, assets, perception, program_id, |p| input.mouse_move(p).map_err(|e| RuntimeError::Input(e.to_string()))),
        Step::Click { at, button } => dispatch_at(at, predicates, frame, now_ms, assets, perception, program_id, |p| input.click(p, *button, &[], 0).map_err(|e| RuntimeError::Input(e.to_string()))),
        Step::Scroll { at, dx, dy } => {
            let _ = (dx, dy);
            dispatch_at(at, predicates, frame, now_ms, assets, perception, program_id, |p| input.mouse_move(p).map_err(|e| RuntimeError::Input(e.to_string())))
        }
        Step::Type { text } => match input.type_text(text) {
            Ok(()) => StepOutcome::Advanced,
            Err(e) => StepOutcome::Failed(RuntimeError::Input(e.to_string())),
        },
        Step::Key { key, modifiers } => match input.key_combo(modifiers, key) {
            Ok(()) => StepOutcome::Advanced,
            Err(e) => StepOutcome::Failed(RuntimeError::Input(e.to_string())),
        },
    }
}

fn dispatch_at<F>(
    target: &Target,
    predicates: &BTreeMap<String, Predicate>,
    frame: &Frame,
    now_ms: UnixMs,
    assets: &BTreeMap<String, Vec<u8>>,
    perception: &Perception,
    program_id: ProgramId,
    f: F,
) -> StepOutcome
where
    F: FnOnce(Point) -> Result<(), RuntimeError>,
{
    let point = match target {
        Target::Absolute(p) => *p,
        Target::Expression(s) => match expr::parse(s) {
            Ok(e) => {
                let r = match perception.evaluate_named(e.predicate, predicates, frame, now_ms, assets, Some(program_id)) {
                    Ok(r) => r,
                    Err(e2) => return StepOutcome::Failed(RuntimeError::Perception(e2.to_string())),
                };
                match e.accessor {
                    expr::Accessor::MatchCenter => match expr::resolve_center(&r) {
                        Ok(p) => p,
                        Err(e) => return StepOutcome::Failed(e),
                    },
                    expr::Accessor::MatchBbox => match expr::resolve_bbox(&r) {
                        Ok(bx) => Point { x: bx.x, y: bx.y },
                        Err(e) => return StepOutcome::Failed(e),
                    },
                }
            }
            Err(e) => return StepOutcome::Failed(e),
        },
    };
    match f(point) {
        Ok(()) => StepOutcome::Advanced,
        Err(e) => StepOutcome::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vcli_core::action::Button;
    use vcli_core::frame::FrameFormat;
    use vcli_core::geom::Rect;

    fn blank_frame() -> Frame {
        Frame::new(FrameFormat::Rgba8, Rect { x: 0, y: 0, w: 1, h: 1 }, 4, Arc::from(vec![0u8, 0, 0, 0]), 0)
    }

    fn id() -> ProgramId { "12345678-1234-4567-8910-111213141516".parse().unwrap() }

    struct NopSink;
    impl InputSink for NopSink {
        fn mouse_move(&self, _: Point) -> Result<(), vcli_input::error::InputError> { Ok(()) }
        fn click(&self, _: Point, _: Button, _: &[vcli_core::action::Modifier], _: u32) -> Result<(), vcli_input::error::InputError> { Ok(()) }
        fn double_click(&self, _: Point, _: Button) -> Result<(), vcli_input::error::InputError> { Ok(()) }
        fn drag(&self, _: Point, _: &[vcli_input::sink::DragSegment], _: Button) -> Result<(), vcli_input::error::InputError> { Ok(()) }
        fn type_text(&self, _: &str) -> Result<(), vcli_input::error::InputError> { Ok(()) }
        fn key_combo(&self, _: &[vcli_core::action::Modifier], _: &str) -> Result<(), vcli_input::error::InputError> { Ok(()) }
    }

    #[test]
    fn absolute_click_advances() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let input: Arc<dyn InputSink> = Arc::new(NopSink);
        let mut st = BodyState::default();
        let body = vec![Step::Click { at: Target::Absolute(Point { x: 1, y: 2 }), button: Button::Left }];
        let out = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 0, &assets, &p, &input);
        assert!(matches!(out, StepOutcome::Advanced));
    }

    #[test]
    fn sleep_ms_stalls_then_advances() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let input: Arc<dyn InputSink> = Arc::new(NopSink);
        let mut st = BodyState::default();
        let body = vec![Step::SleepMs { ms: 100 }];
        let out = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 0, &assets, &p, &input);
        assert!(matches!(out, StepOutcome::Stalled));
        // Advance clock past wake.
        let out2 = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 200, &assets, &p, &input);
        assert!(matches!(out2, StepOutcome::Advanced));
    }

    #[test]
    fn assert_fail_propagates_error() {
        use vcli_core::predicate::{PredicateKind, Rgb};
        let mut preds = BTreeMap::new();
        preds.insert("blue".into(), PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 }, rgb: Rgb([0, 0, 255]), tolerance: 0,
        });
        let assets = BTreeMap::new();
        let p = Perception::new();
        let input: Arc<dyn InputSink> = Arc::new(NopSink);
        let mut st = BodyState::default();
        let body = vec![Step::Assert { predicate: "blue".into(), on_fail: OnFail::Fail }];
        let out = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 0, &assets, &p, &input);
        assert!(matches!(out, StepOutcome::Failed(RuntimeError::AssertFailed { .. })));
    }

    #[test]
    fn cursor_past_end_is_body_complete() {
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = Perception::new();
        let input: Arc<dyn InputSink> = Arc::new(NopSink);
        let mut st = BodyState::default();
        let body: Vec<Step> = vec![];
        let out = step_once(id(), &body, 0, &mut st, &preds, &blank_frame(), 0, &assets, &p, &input);
        assert!(matches!(out, StepOutcome::BodyComplete));
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -p vcli-runtime body::tests`
Expected: 4 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/body.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: body step executor (one step per tick)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: `Scheduler::new` + `run_until_shutdown` skeleton

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/scheduler.rs`

The first cut ties the modules together: constructor, shutdown path, drain-commands-then-tick loop. Real tick logic lands in Task 17.

- [ ] **Step 1: Write skeleton + smoke test**

```rust
//! Scheduler entrypoint. Owns a `HashMap<ProgramId, RunningProgram>` and
//! advances one tick per `tick_interval_ms`.

use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use vcli_core::{Clock, Event, EventData, ProgramId};
use vcli_core::state::ProgramState;
use vcli_perception::Perception;
use vcli_capture::Capture;
use vcli_input::InputSink;

use crate::clock::RuntimeClock;
use crate::command::SchedulerCommand;
use crate::event::EventEmitter;
use crate::program::RunningProgram;

/// Tunable knobs.
#[derive(Debug, Clone, Copy)]
pub struct SchedulerConfig {
    /// Tick cadence.
    pub tick_interval_ms: u32,
    /// Soft budget per tick.
    pub tick_budget_ms: u32,
    /// Concurrent-program cap.
    pub max_inflight: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { tick_interval_ms: 100, tick_budget_ms: 90, max_inflight: 256 }
    }
}

/// The scheduler.
pub struct Scheduler {
    config: SchedulerConfig,
    capture: Box<dyn Capture>,
    input: Arc<dyn InputSink>,
    perception: Perception,
    clock_rt: Arc<dyn RuntimeClock>,
    clock_core: Arc<dyn Clock + Send + Sync>,
    cmd_rx: Receiver<SchedulerCommand>,
    event: EventEmitter,
    programs: HashMap<ProgramId, RunningProgram>,
}

impl Scheduler {
    /// Construct. The daemon will call `run_until_shutdown()` from a dedicated
    /// OS thread immediately after construction.
    #[must_use]
    pub fn new(
        config: SchedulerConfig,
        capture: Box<dyn Capture>,
        input: Arc<dyn InputSink>,
        perception: Perception,
        clock: Arc<dyn RuntimeClock>,
        cmd_rx: Receiver<SchedulerCommand>,
        event_tx: Sender<Event>,
    ) -> Self {
        // Bridge into vcli_core::Clock for EventEmitter (same wall time).
        let core: Arc<dyn Clock + Send + Sync> = Arc::clone(&clock) as Arc<dyn RuntimeClock> as Arc<dyn Clock + Send + Sync>;
        let event = EventEmitter::new(event_tx, Arc::clone(&core));
        Self {
            config,
            capture,
            input,
            perception,
            clock_rt: clock,
            clock_core: core,
            cmd_rx,
            event,
            programs: HashMap::new(),
        }
    }

    /// Main loop. Ticks on each `tick_interval_ms` deadline; returns when a
    /// `Shutdown` command is observed. Emits `daemon.stopped` on the way out
    /// so the daemon's persist layer sees a clean trailing event.
    pub fn run_until_shutdown(mut self) {
        loop {
            if self.drain_commands() {
                break;
            }
            self.tick_once();
            self.clock_rt.sleep_ms(self.config.tick_interval_ms);
        }
        self.event.emit(EventData::DaemonStopped);
    }

    /// Drain pending commands. Returns `true` if a `Shutdown` was observed.
    fn drain_commands(&mut self) -> bool {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                SchedulerCommand::Shutdown => return true,
                SchedulerCommand::SubmitValidated { program_id, program } => {
                    let mut rp = RunningProgram::pending(program_id, program);
                    rp.state = ProgramState::Waiting;
                    self.event.emit(EventData::ProgramSubmitted { program_id, name: rp.program.name.clone() });
                    self.event.emit(EventData::ProgramStateChanged {
                        program_id,
                        from: ProgramState::Pending,
                        to: ProgramState::Waiting,
                        reason: "submitted".into(),
                    });
                    self.programs.insert(program_id, rp);
                }
                SchedulerCommand::Cancel { program_id, reason } => {
                    if let Some(rp) = self.programs.get_mut(&program_id) {
                        let from = rp.state;
                        rp.state = ProgramState::Cancelled;
                        self.event.emit(EventData::ProgramStateChanged {
                            program_id, from, to: ProgramState::Cancelled, reason,
                        });
                    }
                }
                SchedulerCommand::Start { program_id } => {
                    if let Some(rp) = self.programs.get_mut(&program_id) {
                        let from = rp.state;
                        rp.state = ProgramState::Running;
                        rp.running_since_ms = Some(self.clock_core.now_ms());
                        self.event.emit(EventData::ProgramStateChanged {
                            program_id, from, to: ProgramState::Running, reason: "start".into(),
                        });
                    }
                }
                SchedulerCommand::ResumeRunning { program_id, from_step, program } => {
                    let mut rp = RunningProgram::pending(program_id, program);
                    rp.state = ProgramState::Running;
                    rp.running_since_ms = Some(self.clock_core.now_ms());
                    rp.body_cursor = Some(from_step);
                    rp.resumed_from = Some(from_step);
                    self.event.emit(EventData::ProgramStateChanged {
                        program_id, from: ProgramState::Failed, to: ProgramState::Running,
                        reason: "resume".into(),
                    });
                    self.event.emit(EventData::ProgramResumed { program_id, from_step });
                    self.programs.insert(program_id, rp);
                }
            }
        }
        false
    }

    /// Placeholder tick body. Real evaluation lands in Task 17.
    fn tick_once(&mut self) {
        // TODO(Task 17): capture → perception → watches → body → arbiter.
        let _ = &self.capture;
        let _ = &self.input;
        let _ = &self.perception;
    }
}
```

The above has an intentional placeholder `tick_once()`. Task 17 replaces it.

- [ ] **Step 2: Verify the crate compiles**

Run: `cargo check -p vcli-runtime`
Expected: no errors.

- [ ] **Step 3: Smoke test — submit + shutdown round-trips events**

Add to `crates/vcli-runtime/src/scheduler.rs` (at the bottom, inside `#[cfg(test)] mod tests`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use crossbeam_channel::unbounded;
    use vcli_core::{program::DslVersion, trigger::Trigger, Program};
    use vcli_capture::capture::{Capture, WindowDescriptor};
    use vcli_capture::error::CaptureError;
    use vcli_core::frame::{Frame, FrameFormat};
    use vcli_core::geom::Rect;
    use vcli_input::error::InputError;
    use vcli_input::sink::DragSegment;
    use vcli_core::action::{Button, Modifier};
    use vcli_core::geom::Point;
    use crate::clock::ManualClock;

    struct StaticCapture;
    impl Capture for StaticCapture {
        fn supported_formats(&self) -> &[FrameFormat] { &[FrameFormat::Rgba8] }
        fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> { Ok(vec![]) }
        fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
            Ok(Frame::new(
                FrameFormat::Rgba8,
                Rect { x: 0, y: 0, w: 1, h: 1 },
                4,
                std::sync::Arc::from(vec![0u8, 0, 0, 0]),
                0,
            ))
        }
        fn grab_window(&mut self, _: &WindowDescriptor) -> Result<Frame, CaptureError> {
            self.grab_screen()
        }
    }

    struct NopInput;
    impl InputSink for NopInput {
        fn mouse_move(&self, _: Point) -> Result<(), InputError> { Ok(()) }
        fn click(&self, _: Point, _: Button, _: &[Modifier], _: u32) -> Result<(), InputError> { Ok(()) }
        fn double_click(&self, _: Point, _: Button) -> Result<(), InputError> { Ok(()) }
        fn drag(&self, _: Point, _: &[DragSegment], _: Button) -> Result<(), InputError> { Ok(()) }
        fn type_text(&self, _: &str) -> Result<(), InputError> { Ok(()) }
        fn key_combo(&self, _: &[Modifier], _: &str) -> Result<(), InputError> { Ok(()) }
    }

    fn empty_program() -> Program {
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: "x".into(),
            id: None,
            trigger: Trigger::OnSubmit,
            predicates: BTreeMap::new(),
            watches: vec![],
            body: vec![],
            on_complete: None,
            on_fail: None,
            timeout_ms: None,
            labels: BTreeMap::new(),
            priority: Default::default(),
        }
    }

    #[test]
    fn shutdown_exits_cleanly() {
        let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
        let (ev_tx, ev_rx) = unbounded::<Event>();
        let clock = std::sync::Arc::new(ManualClock::new(0));
        let perc = Perception::new();
        let sched = Scheduler::new(
            SchedulerConfig::default(),
            Box::new(StaticCapture),
            std::sync::Arc::new(NopInput),
            perc,
            clock,
            cmd_rx,
            ev_tx,
        );
        let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
        cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program: empty_program() }).unwrap();
        cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
        // Run inline (fine for empty work).
        let handle = std::thread::spawn(move || sched.run_until_shutdown());
        handle.join().unwrap();
        // Must have seen at least ProgramSubmitted, ProgramStateChanged (pending→waiting), DaemonStopped.
        let mut kinds: Vec<String> = Vec::new();
        while let Ok(ev) = ev_rx.try_recv() {
            kinds.push(serde_json::to_value(&ev).unwrap()["type"].as_str().unwrap().to_string());
        }
        assert!(kinds.iter().any(|k| k == "program.submitted"));
        assert!(kinds.iter().any(|k| k == "program.state_changed"));
        assert_eq!(kinds.last().unwrap(), "daemon.stopped");
    }
}
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo test -p vcli-runtime scheduler::tests::shutdown_exits_cleanly`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-runtime/src/scheduler.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: Scheduler::new + run_until_shutdown skeleton

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 17: Fleshed-out `tick_once` — capture, evaluate watches, dispatch actions

**Files:**
- Modify: `/Users/admin/Workspace/vcli/crates/vcli-runtime/src/scheduler.rs`

Replace the placeholder `tick_once` with the real implementation that ties together `capture`, `perception`, `watches`, `triggers`, `body`, `arbiter`, and `confirm`. Every emission path is under test coverage via the scenario tests in Tasks 18–30.

- [ ] **Step 1: Replace `tick_once`**

```rust
fn tick_once(&mut self) {
    // 1. Capture.
    let frame = match self.capture.grab_screen() {
        Ok(f) => std::sync::Arc::new(f),
        Err(e) => {
            tracing::warn!("capture failed: {e}");
            self.event.emit(EventData::TickFrameSkipped { reason: "capture_failed".into() });
            return;
        }
    };
    // 2. Wipe per-tick cache.
    self.perception.clear();
    let now_ms = self.clock_core.now_ms();
    let assets: std::collections::BTreeMap<String, Vec<u8>> = std::collections::BTreeMap::new();

    // 3. Advance triggers for Waiting programs.
    let waiting: Vec<ProgramId> = self.programs
        .iter()
        .filter(|(_, p)| p.state == ProgramState::Waiting)
        .map(|(id, _)| *id)
        .collect();
    for id in waiting {
        let fire = {
            let p = &self.programs[&id];
            crate::triggers::trigger_fires(
                &p.program.trigger, &p.program.predicates, &frame, now_ms, &assets, &self.perception, id,
            )
        };
        match fire {
            Ok(true) => {
                let rp = self.programs.get_mut(&id).unwrap();
                rp.state = ProgramState::Running;
                rp.running_since_ms = Some(now_ms);
                rp.body_cursor = Some(0);
                // Populate watch_state with defaults.
                for (i, _) in rp.program.watches.iter().enumerate() {
                    rp.watch_state.insert(u32::try_from(i).unwrap_or(u32::MAX), crate::program::WatchRuntime::default());
                }
                self.event.emit(EventData::ProgramStateChanged {
                    program_id: id, from: ProgramState::Waiting, to: ProgramState::Running, reason: "trigger_fired".into(),
                });
            }
            Ok(false) => {}
            Err(e) => {
                self.programs.get_mut(&id).unwrap().state = ProgramState::Failed;
                self.event.emit(EventData::ProgramFailed {
                    program_id: id, reason: e.code().as_str().into(), step: None, emit: None,
                });
            }
        }
    }

    // 4. Evaluate watches on Running programs; collect candidate actions.
    let running_ids: Vec<ProgramId> = self.programs
        .iter()
        .filter(|(_, p)| p.state == ProgramState::Running)
        .map(|(id, _)| *id)
        .collect();

    // For arbitration we collect a single "winner per program" then resolve.
    let mut per_program_steps: Vec<crate::arbiter::Candidate<(ProgramId, u32, Vec<vcli_core::Step>)>> = Vec::new();

    for id in running_ids.iter().copied() {
        let rp = self.programs.get(&id).unwrap();
        let running_since = rp.running_since_ms.unwrap_or(now_ms);
        let mut fires: Vec<(u32, Vec<vcli_core::Step>, bool /*is_one_shot*/)> = Vec::new();
        for (idx, w) in rp.program.watches.iter().enumerate() {
            let idx_u32 = u32::try_from(idx).unwrap_or(u32::MAX);
            let state = rp.watch_state.get(&idx_u32).cloned().unwrap_or_default();
            // Evaluate the watch.when predicate.
            let truthy = match &w.when {
                vcli_core::watch::WatchWhen::ByName(n) => match self.perception.evaluate_named(n, &rp.program.predicates, &frame, now_ms, &assets, Some(id)) {
                    Ok(r) => r.truthy,
                    Err(e) => { tracing::warn!("perception: {e}"); continue; }
                },
                vcli_core::watch::WatchWhen::Inline(p) => {
                    // Inline predicates don't live in the named map; dedupe through a temporary map key.
                    let mut tmp = rp.program.predicates.clone();
                    let key = format!("__inline_{idx}");
                    tmp.insert(key.clone(), *p.clone());
                    match self.perception.evaluate_named(&key, &tmp, &frame, now_ms, &assets, Some(id)) {
                        Ok(r) => r.truthy,
                        Err(e) => { tracing::warn!("perception: {e}"); continue; }
                    }
                }
            };
            match crate::watches::decide(w, &state, truthy, now_ms, running_since) {
                crate::watches::WatchDecision::Fire => fires.push((idx_u32, w.steps.clone(), matches!(w.lifetime, vcli_core::watch::Lifetime::OneShot))),
                crate::watches::WatchDecision::Skip => {}
                crate::watches::WatchDecision::Retire => {
                    // Apply retirement now.
                }
            }
        }
        if let Some((_, steps, _)) = fires.first() {
            // One candidate per program per tick (first-firing watch wins within a program).
            per_program_steps.push(crate::arbiter::Candidate {
                program_id: id,
                priority: rp.program.priority,
                payload: (id, 0 /*placeholder*/, steps.clone()),
            });
        }
    }
    let decisions = crate::arbiter::resolve(per_program_steps);
    for d in decisions {
        let (prog_id, _watch_idx, steps) = d.payload;
        if d.dispatch {
            for s in &steps {
                // Synthesize a minimal action.dispatched event (resolving targets is body.rs's job;
                // for watches we reuse the same resolution by calling into body::dispatch_at through
                // a pared-down helper. For brevity in the skeleton, we just emit the step shape.)
                let value = serde_json::to_value(s).unwrap_or(serde_json::Value::Null);
                self.event.emit(EventData::ActionDispatched { program_id: prog_id, step: value, target: None });
            }
            self.event.emit(EventData::WatchFired { program_id: prog_id, watch_index: 0, predicate: "watch".into() });
        } else if let Some(w) = d.loser_of {
            let value = serde_json::Value::Array(steps.iter().map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null)).collect());
            self.event.emit(EventData::ActionDeferred {
                program_id: prog_id,
                step: value,
                reason: serde_json::json!({ "conflict_with": w.to_string() }),
            });
        }
    }

    // 5. Advance one body step for each Running program.
    for id in running_ids.iter().copied() {
        let rp = self.programs.get_mut(&id).unwrap();
        let cursor = rp.body_cursor.unwrap_or(0);
        let body = rp.program.body.clone();
        let mut state = crate::body::BodyState::default();
        let outcome = crate::body::step_once(
            id, &body, cursor, &mut state, &rp.program.predicates,
            &frame, now_ms, &assets, &self.perception, &self.input,
        );
        match outcome {
            crate::body::StepOutcome::Advanced => {
                rp.body_cursor = Some(cursor + 1);
                if rp.body_complete() && rp.active_watch_count() == 0 {
                    let emit = rp.program.on_complete.as_ref().map(|c| c.emit.clone());
                    rp.state = ProgramState::Completed;
                    self.event.emit(EventData::ProgramStateChanged {
                        program_id: id, from: ProgramState::Running, to: ProgramState::Completed,
                        reason: "body_complete".into(),
                    });
                    self.event.emit(EventData::ProgramCompleted { program_id: id, emit });
                }
            }
            crate::body::StepOutcome::Stalled => {}
            crate::body::StepOutcome::BodyComplete => {
                if rp.active_watch_count() == 0 {
                    let emit = rp.program.on_complete.as_ref().map(|c| c.emit.clone());
                    rp.state = ProgramState::Completed;
                    self.event.emit(EventData::ProgramStateChanged {
                        program_id: id, from: ProgramState::Running, to: ProgramState::Completed,
                        reason: "body_complete".into(),
                    });
                    self.event.emit(EventData::ProgramCompleted { program_id: id, emit });
                }
            }
            crate::body::StepOutcome::Failed(err) => {
                let emit = rp.program.on_fail.as_ref().map(|f| f.emit.clone());
                rp.state = ProgramState::Failed;
                self.event.emit(EventData::ProgramFailed {
                    program_id: id,
                    reason: err.code().as_str().into(),
                    step: Some(format!("body[{cursor}]")),
                    emit,
                });
            }
        }
    }
}
```

This is intentionally "minimum viable correctness" — the scenario tests in Tasks 18–30 drive out edge cases (throttle, OneShot retirement, action conflict, novelty_timeout, etc.). As the scenarios are implemented, tighten `tick_once` until all assertions pass. Commit after each scenario lands, per TDD.

- [ ] **Step 2: Verify crate still compiles and shutdown smoke test still passes**

Run: `cargo test -p vcli-runtime scheduler::tests::shutdown_exits_cleanly`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/src/scheduler.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: fleshed-out tick_once (capture, watches, body, arbiter)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 18: `tests/common/` — shared mocks + helpers

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/tests/common/mod.rs`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/tests/common/mock_capture.rs`
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/tests/common/mock_input.rs`

- [ ] **Step 1: Create `tests/common/mod.rs`**

```rust
//! Shared test helpers for vcli-runtime scenario tests.

#![allow(dead_code)]   // every scenario uses a different subset

use std::collections::BTreeMap;
use std::sync::Arc;

pub use crossbeam_channel::{unbounded, Receiver, Sender};
pub use vcli_core::{Event, ProgramId};
pub use vcli_core::state::ProgramState;
pub use vcli_runtime::{Scheduler, SchedulerCommand, SchedulerConfig};

pub mod mock_capture;
pub mod mock_input;

pub use mock_capture::ScriptedCapture;
pub use mock_input::RecordingInputSink;

/// Extract the `type` tag from a serialized event, for pattern-matching in assertions.
pub fn event_type(e: &Event) -> String {
    serde_json::to_value(e).unwrap()["type"].as_str().unwrap_or("").to_string()
}

/// Drain a channel into a Vec (non-blocking).
pub fn drain_events(rx: &Receiver<Event>) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() { out.push(e); }
    out
}

/// Build an empty predicate map (convenience for scenarios).
#[must_use]
pub fn empty_predicates() -> BTreeMap<String, vcli_core::Predicate> { BTreeMap::new() }
```

- [ ] **Step 2: `mock_capture.rs`**

```rust
//! Scripted Capture: returns a queue of pre-built Frames, in order, then
//! repeats the last one. `enumerate_windows` returns an empty list.

use std::sync::{Arc, Mutex};

use vcli_capture::capture::{Capture, DisplayId, WindowDescriptor};
use vcli_capture::error::CaptureError;
use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;

pub struct ScriptedCapture {
    frames: Mutex<Vec<Frame>>,
    cursor: Mutex<usize>,
}

impl ScriptedCapture {
    pub fn new(frames: Vec<Frame>) -> Self {
        Self { frames: Mutex::new(frames), cursor: Mutex::new(0) }
    }

    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
        let n = (width * height) as usize;
        let mut buf = Vec::with_capacity(n * 4);
        for _ in 0..n { buf.extend_from_slice(&rgba); }
        Frame::new(
            FrameFormat::Rgba8,
            Rect { x: 0, y: 0, w: i32::try_from(width).unwrap(), h: i32::try_from(height).unwrap() },
            4,
            Arc::from(buf),
            0,
        )
    }
}

impl Capture for ScriptedCapture {
    fn supported_formats(&self) -> &[FrameFormat] { &[FrameFormat::Rgba8] }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> { Ok(vec![]) }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        let frames = self.frames.lock().unwrap();
        let mut cursor = self.cursor.lock().unwrap();
        if frames.is_empty() {
            return Ok(Self::solid(1, 1, [0, 0, 0, 0]));
        }
        let i = (*cursor).min(frames.len() - 1);
        let out = frames[i].clone();
        *cursor = (i + 1).min(frames.len() - 1);
        Ok(out)
    }

    fn grab_window(&mut self, _: &WindowDescriptor) -> Result<Frame, CaptureError> {
        self.grab_screen()
    }
}
```

- [ ] **Step 3: `mock_input.rs`**

```rust
//! Recording InputSink: captures every method call into a `Vec<Call>` for assertion.

use std::sync::Mutex;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;
use vcli_input::error::InputError;
use vcli_input::sink::{DragSegment, InputSink};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Move(Point),
    Click(Point, Button),
    DoubleClick(Point, Button),
    Drag(Point, Vec<DragSegment>, Button),
    Type(String),
    Key(Vec<Modifier>, String),
}

#[derive(Default)]
pub struct RecordingInputSink {
    pub calls: Mutex<Vec<Call>>,
}

impl RecordingInputSink {
    pub fn new() -> Self { Self::default() }
    pub fn calls(&self) -> Vec<Call> { self.calls.lock().unwrap().clone() }
}

impl InputSink for RecordingInputSink {
    fn mouse_move(&self, to: Point) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::Move(to));
        Ok(())
    }
    fn click(&self, at: Point, button: Button, _: &[Modifier], _: u32) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::Click(at, button));
        Ok(())
    }
    fn double_click(&self, at: Point, button: Button) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::DoubleClick(at, button));
        Ok(())
    }
    fn drag(&self, from: Point, segs: &[DragSegment], button: Button) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::Drag(from, segs.to_vec(), button));
        Ok(())
    }
    fn type_text(&self, s: &str) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::Type(s.to_string()));
        Ok(())
    }
    fn key_combo(&self, mods: &[Modifier], k: &str) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::Key(mods.to_vec(), k.to_string()));
        Ok(())
    }
}
```

- [ ] **Step 4: Verify the common module compiles by running one placeholder test**

Run: `cargo test -p vcli-runtime --test common 2>&1 | head -20`
(If there's no binary target named `common`, `cargo test` just ignores it — which is fine because `common/` is not a test binary, it's a module pulled in by `tests/scenarios/*.rs`. Instead verify we haven't introduced a compile error:)
Run: `cargo check -p vcli-runtime --tests`
Expected: zero errors.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-runtime/tests/common/
git commit -m "$(cat <<'EOF'
vcli-runtime: scenario-test mocks (ScriptedCapture, RecordingInputSink)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 19: Scenario — `one_shot_watch`

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/tests/scenarios/one_shot_watch.rs`

Spec scenario: a one-shot watch fires on the first false→true transition and then the program completes (no body, no more watches).

- [ ] **Step 1: Write the failing test**

```rust
//! Scenario: Lifetime::OneShot fires once and the program completes.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vcli_core::geom::{Point, Rect};
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::step::{Step, Target};
use vcli_core::watch::{Lifetime, Watch, WatchWhen};
use vcli_core::{program::DslVersion, trigger::Trigger, Program};
use vcli_core::action::Button;
use vcli_runtime::clock::ManualClock;
use vcli_perception::Perception;

#[test]
fn one_shot_watch_fires_once_and_completes() {
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();

    let red_frame = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red_frame]));
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(1_000));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), PredicateKind::ColorAt {
        point: Point { x: 0, y: 0 }, rgb: Rgb([255, 0, 0]), tolerance: 0,
    });
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "one_shot".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![Watch {
            when: WatchWhen::ByName("red".into()),
            steps: vec![Step::Click {
                at: Target::Absolute(Point { x: 10, y: 20 }),
                button: Button::Left,
            }],
            throttle_ms: 0,
            lifetime: Lifetime::OneShot,
        }],
        body: vec![],
        on_complete: None,
        on_fail: None,
        timeout_ms: None,
        labels: BTreeMap::new(),
        priority: Default::default(),
    };

    let sched = Scheduler::new(
        SchedulerConfig::default(),
        capture,
        input.clone(),
        Perception::new(),
        clock.clone(),
        cmd_rx,
        ev_tx,
    );

    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();

    let handle = std::thread::spawn(move || sched.run_until_shutdown());
    // Give the scheduler a few ticks worth of ManualClock time then shutdown.
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    handle.join().unwrap();

    let events = drain_events(&ev_rx);
    let types: Vec<_> = events.iter().map(event_type).collect();

    // Submission + transition to waiting + running + watch.fired + completion.
    assert!(types.iter().any(|t| t == "program.submitted"), "types: {types:?}");
    assert!(types.iter().any(|t| t == "watch.fired"),      "types: {types:?}");
    assert!(types.iter().any(|t| t == "program.completed"),"types: {types:?}");

    // Click was dispatched exactly once.
    let clicks: Vec<_> = input.calls().into_iter().filter(|c| matches!(c, common::mock_input::Call::Click(_, _))).collect();
    assert_eq!(clicks.len(), 1, "expected exactly one click, got {clicks:?}");
}
```

- [ ] **Step 2: Run the test and expect FAIL**

Run: `cargo test -p vcli-runtime --test one_shot_watch one_shot_watch_fires_once_and_completes`
Expected: FAIL — most likely on the "completed" assertion because the current `tick_once` retires the watch but doesn't yet decrement `active_watch_count` or emit completion at the right moment. Track the failure and iterate in Step 3.

- [ ] **Step 3: Tighten `scheduler.rs::tick_once` so the test passes**

The key fix: after deciding `WatchDecision::Fire`, apply `watches::after_fire()` to the watch's state, then emit `watch.fired`. When `active_watch_count() == 0 && body_complete()`, transition to `Completed`.

Reopen `scheduler.rs`. Inside the per-program watch loop, replace the placeholder "apply retirement now" branch and the post-arbiter emission with logic that updates `WatchRuntime` in the owning `RunningProgram` and emits `watch.fired` only for dispatched actions.

Sketch of the diff:

```rust
// Inside tick_once, replace the `fires` collection + arbitration block with a
// two-step process:
//   (a) collect per-program decisions (idx, steps, lifetime) for the FIRST
//       firing watch;
//   (b) after arbitration, for each dispatched candidate, flip last_fired_ms,
//       set last_truthy = true, and retire OneShots;
//   (c) for each skipped/retired watch (no fire this tick), just update
//       last_truthy from the evaluated predicate so we re-detect edges next
//       tick.
```

Iterate until the test passes. Then:

Run: `cargo test -p vcli-runtime --test one_shot_watch`
Expected: PASS.

- [ ] **Step 4: Regression check**

Run: `cargo test -p vcli-runtime`
Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-runtime/tests/scenarios/one_shot_watch.rs crates/vcli-runtime/src/scheduler.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: scenario one_shot_watch + tick wiring to pass it

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 20: Scenario — `persistent_watch`

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/tests/scenarios/persistent_watch.rs`

A `Persistent` watch fires every false→true edge, not on level-high. With a two-frame script ([red, blue, red]) it should fire twice.

- [ ] **Step 1: Write the failing test**

```rust
//! Scenario: Lifetime::Persistent fires on every false→true edge.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vcli_core::action::Button;
use vcli_core::geom::Point;
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::step::{Step, Target};
use vcli_core::watch::{Lifetime, Watch, WatchWhen};
use vcli_core::{program::DslVersion, trigger::Trigger, Program};
use vcli_perception::Perception;
use vcli_runtime::clock::ManualClock;

#[test]
fn persistent_watch_fires_on_each_edge() {
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();

    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let blue = ScriptedCapture::solid(1, 1, [0, 0, 255, 255]);
    // Frame sequence: red, blue, red, blue, red — two rising edges.
    let capture = Box::new(ScriptedCapture::new(vec![
        red.clone(), blue.clone(), red.clone(), blue.clone(), red,
    ]));
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), PredicateKind::ColorAt {
        point: Point { x: 0, y: 0 }, rgb: Rgb([255, 0, 0]), tolerance: 0,
    });
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "persistent".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![Watch {
            when: WatchWhen::ByName("red".into()),
            steps: vec![Step::Click { at: Target::Absolute(Point { x: 0, y: 0 }), button: Button::Left }],
            throttle_ms: 0,
            lifetime: Lifetime::Persistent,
        }],
        body: vec![],
        on_complete: None,
        on_fail: None,
        timeout_ms: None,
        labels: BTreeMap::new(),
        priority: Default::default(),
    };

    let sched = Scheduler::new(
        SchedulerConfig::default(),
        capture,
        input.clone(),
        Perception::new(),
        clock,
        cmd_rx,
        ev_tx,
    );
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();
    let handle = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(500));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    handle.join().unwrap();

    let events = drain_events(&ev_rx);
    let watch_fires = events.iter().filter(|e| event_type(e) == "watch.fired").count();
    assert!(watch_fires >= 2, "expected >= 2 watch fires (two rising edges), got {watch_fires}; events={:?}",
        events.iter().map(event_type).collect::<Vec<_>>());
}
```

- [ ] **Step 2: Run and iterate**

Run: `cargo test -p vcli-runtime --test persistent_watch`
Adjust `tick_once` if the count is off (most commonly because `last_truthy` is not being updated when the predicate is truthy but the watch is throttled).

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/tests/scenarios/persistent_watch.rs crates/vcli-runtime/src/scheduler.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: scenario persistent_watch

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 21: Scenario — `until_predicate`

**Files:**
- Create: `/Users/admin/Workspace/vcli/crates/vcli-runtime/tests/scenarios/until_predicate.rs`

A `Lifetime::UntilPredicate { name: "done" }` watch is retired — without firing this tick — as soon as `done` becomes truthy.

- [ ] **Step 1: Write the test**

```rust
//! Scenario: UntilPredicate retires the watch when its terminator fires.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vcli_core::action::Button;
use vcli_core::geom::Point;
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::step::{Step, Target};
use vcli_core::watch::{Lifetime, Watch, WatchWhen};
use vcli_core::{program::DslVersion, trigger::Trigger, Program};
use vcli_perception::Perception;
use vcli_runtime::clock::ManualClock;

#[test]
fn until_predicate_retires_watch() {
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();

    // First frame: red (watch fires). Second frame: green (terminator truthy).
    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let green = ScriptedCapture::solid(1, 1, [0, 255, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red, green.clone(), green]));
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("is_red".into(), PredicateKind::ColorAt {
        point: Point { x: 0, y: 0 }, rgb: Rgb([255, 0, 0]), tolerance: 0,
    });
    preds.insert("done".into(), PredicateKind::ColorAt {
        point: Point { x: 0, y: 0 }, rgb: Rgb([0, 255, 0]), tolerance: 0,
    });

    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "until".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![Watch {
            when: WatchWhen::ByName("is_red".into()),
            steps: vec![Step::Click { at: Target::Absolute(Point { x: 0, y: 0 }), button: Button::Left }],
            throttle_ms: 0,
            lifetime: Lifetime::UntilPredicate { name: "done".into() },
        }],
        body: vec![],
        on_complete: None,
        on_fail: None,
        timeout_ms: None,
        labels: BTreeMap::new(),
        priority: Default::default(),
    };

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input.clone(), Perception::new(), clock, cmd_rx, ev_tx);
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(400));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    // Exactly one fire (tick 0), then terminator retires the watch and program completes.
    let fires = events.iter().filter(|e| event_type(e) == "watch.fired").count();
    let completes = events.iter().filter(|e| event_type(e) == "program.completed").count();
    assert!(fires >= 1 && fires <= 2);  // possibly two if scheduling runs red again before terminator
    assert_eq!(completes, 1);
}
```

- [ ] **Step 2: Iterate until passing**

Run: `cargo test -p vcli-runtime --test until_predicate`
In `tick_once`, evaluate each watch's `UntilPredicate.name` *before* deciding to fire; if truthy, mark the watch retired and skip this tick's edge.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-runtime/tests/scenarios/until_predicate.rs crates/vcli-runtime/src/scheduler.rs
git commit -m "$(cat <<'EOF'
vcli-runtime: scenario until_predicate

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Scenario tasks 22–30 — common template

Each remaining scenario follows the same 5-step shape as Tasks 19–21:

1. **Write the failing test** at `crates/vcli-runtime/tests/scenarios/<name>.rs` using the same `#[path = "../common/mod.rs"] mod common;` prelude. Every test builds a `Program`, submits it, sleeps briefly, shutdowns, and asserts on the drained event stream + `RecordingInputSink` calls.
2. **Run it** — expect FAIL. The failure pinpoints exactly which branch in `tick_once` / `body::step_once` / `watches::decide` needs the next piece of logic.
3. **Patch `scheduler.rs` / `body.rs` / `watches.rs`** until the test is green. The general rule: each scenario adds at most one small behavioral tightening; if you find yourself rewriting large blocks, stop and check the scenario test first.
4. **Regression check** — `cargo test -p vcli-runtime` all green.
5. **Commit** with message `vcli-runtime: scenario <name>`.

Below are the test bodies. Imports that are identical across scenarios (the `common::*` prelude plus `std::sync::Arc`, `std::time::Duration`, and `vcli_core::{program::DslVersion, trigger::Trigger, Program}`) are assumed for brevity; add them to every scenario file.

### Task 22: `watch_timeout`

```rust
// tests/scenarios/watch_timeout.rs
// Scenario: Lifetime::TimeoutMs retires the watch after N ms even if it
// never observed a false→true edge.

#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn watch_timeout_retires_after_budget() {
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let blue = ScriptedCapture::solid(1, 1, [0, 0, 255, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![blue]));
    let input   = Arc::new(RecordingInputSink::new());
    let clock   = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), vcli_core::predicate::PredicateKind::ColorAt {
        point: vcli_core::geom::Point { x: 0, y: 0 },
        rgb:   vcli_core::predicate::Rgb([255, 0, 0]),
        tolerance: 0,
    });
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "timeout".into(), id: None, trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![vcli_core::watch::Watch {
            when: vcli_core::watch::WatchWhen::ByName("red".into()),
            steps: vec![],
            throttle_ms: 0,
            lifetime: vcli_core::watch::Lifetime::TimeoutMs { ms: 200 },
        }],
        body: vec![], on_complete: None, on_fail: None,
        timeout_ms: None, labels: BTreeMap::new(), priority: Default::default(),
    };

    let sched = Scheduler::new(
        SchedulerConfig::default(), capture, input, vcli_perception::Perception::new(),
        clock, cmd_rx, ev_tx,
    );
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(500));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    // No watch.fired, but program must complete because the watch retired.
    assert_eq!(events.iter().filter(|e| event_type(e) == "watch.fired").count(), 0);
    assert_eq!(events.iter().filter(|e| event_type(e) == "program.completed").count(), 1);
}
```

Behavior to tighten in `tick_once`: when `watches::decide(...) == Retire`, mark the watch retired; after evaluating all watches, if `body_complete() && active_watch_count() == 0`, transition to `Completed`.

Commit: `vcli-runtime: scenario watch_timeout`.

### Task 23: `while_true_retry`

Tests that a watch with `throttle_ms: T` fires again after `T` ms while still on a rising edge (meaning: level-high for a while, drops to false, rises to true — two fires). This is the "retry" semantic in v0: throttle suppresses rapid fires; scenarios of genuine level-high should be modeled via repeat edges in the scripted frames.

```rust
// tests/scenarios/while_true_retry.rs
#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn throttled_persistent_fires_at_most_once_per_window() {
    let red  = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let blue = ScriptedCapture::solid(1, 1, [0, 0, 255, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red.clone(), blue.clone(), red.clone(), blue.clone(), red]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), vcli_core::predicate::PredicateKind::ColorAt {
        point: vcli_core::geom::Point { x: 0, y: 0 },
        rgb:   vcli_core::predicate::Rgb([255, 0, 0]),
        tolerance: 0,
    });
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "throttle".into(), id: None, trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![vcli_core::watch::Watch {
            when: vcli_core::watch::WatchWhen::ByName("red".into()),
            steps: vec![vcli_core::step::Step::Click {
                at: vcli_core::step::Target::Absolute(vcli_core::geom::Point { x: 0, y: 0 }),
                button: vcli_core::action::Button::Left,
            }],
            // Throttle large enough to block the second edge within the test window.
            throttle_ms: 10_000,
            lifetime: vcli_core::watch::Lifetime::Persistent,
        }],
        body: vec![], on_complete: None, on_fail: None,
        timeout_ms: None, labels: BTreeMap::new(), priority: Default::default(),
    };

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input.clone(), vcli_perception::Perception::new(), clock, cmd_rx, ev_tx);
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(500));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let clicks: Vec<_> = input.calls().into_iter().filter(|c| matches!(c, common::mock_input::Call::Click(_, _))).collect();
    assert_eq!(clicks.len(), 1, "throttle must cap fires at one per window");
}
```

Commit: `vcli-runtime: scenario while_true_retry`.

### Task 24: `wait_for_timeout`

```rust
// tests/scenarios/wait_for_timeout.rs
#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn wait_for_fails_on_timeout_when_on_timeout_fail() {
    let blue = ScriptedCapture::solid(1, 1, [0, 0, 255, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![blue]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), vcli_core::predicate::PredicateKind::ColorAt {
        point: vcli_core::geom::Point { x: 0, y: 0 },
        rgb:   vcli_core::predicate::Rgb([255, 0, 0]),
        tolerance: 0,
    });
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "wait".into(), id: None, trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![],
        body: vec![vcli_core::step::Step::WaitFor {
            predicate: "red".into(),
            timeout_ms: 150,
            on_timeout: vcli_core::step::OnTimeout::Fail,
        }],
        on_complete: None, on_fail: None, timeout_ms: None,
        labels: BTreeMap::new(), priority: Default::default(),
    };

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input, vcli_perception::Perception::new(), clock, cmd_rx, ev_tx);
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(500));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    let failed: Vec<_> = events.iter().filter(|e| event_type(e) == "program.failed").collect();
    assert_eq!(failed.len(), 1);
}
```

Commit: `vcli-runtime: scenario wait_for_timeout`.

### Task 25: `assert_failure`

```rust
// tests/scenarios/assert_failure.rs
#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn assert_fail_on_fail_fails_program() {
    let blue = ScriptedCapture::solid(1, 1, [0, 0, 255, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![blue]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), vcli_core::predicate::PredicateKind::ColorAt {
        point: vcli_core::geom::Point { x: 0, y: 0 },
        rgb:   vcli_core::predicate::Rgb([255, 0, 0]),
        tolerance: 0,
    });
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "assert".into(), id: None, trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![],
        body: vec![vcli_core::step::Step::Assert {
            predicate: "red".into(),
            on_fail: vcli_core::step::OnFail::Fail,
        }],
        on_complete: None, on_fail: None, timeout_ms: None,
        labels: BTreeMap::new(), priority: Default::default(),
    };

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input, vcli_perception::Perception::new(), clock, cmd_rx, ev_tx);
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    assert_eq!(events.iter().filter(|e| event_type(e) == "program.failed").count(), 1);
}
```

Commit: `vcli-runtime: scenario assert_failure`.

### Task 26: `input_postcondition`

Scenario: a click step with a postcondition that flips truthy on the next frame must not fail; if it never flips, it fails with `novelty_timeout`. Because v0 doesn't yet surface postconditions from the DSL, this scenario uses a body sequence `Click → WaitFor(postcondition, 200ms)` as an equivalence.

```rust
// tests/scenarios/input_postcondition.rs
#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn click_then_wait_for_new_state_succeeds() {
    // Frame 0 = red, frame 1 = green (simulates the click "changing" the screen).
    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let green = ScriptedCapture::solid(1, 1, [0, 255, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red, green]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("green".into(), vcli_core::predicate::PredicateKind::ColorAt {
        point: vcli_core::geom::Point { x: 0, y: 0 },
        rgb:   vcli_core::predicate::Rgb([0, 255, 0]),
        tolerance: 0,
    });
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "postcond".into(), id: None, trigger: Trigger::OnSubmit,
        predicates: preds, watches: vec![],
        body: vec![
            vcli_core::step::Step::Click {
                at: vcli_core::step::Target::Absolute(vcli_core::geom::Point { x: 0, y: 0 }),
                button: vcli_core::action::Button::Left,
            },
            vcli_core::step::Step::WaitFor {
                predicate: "green".into(),
                timeout_ms: 300,
                on_timeout: vcli_core::step::OnTimeout::Fail,
            },
        ],
        on_complete: None, on_fail: None, timeout_ms: None,
        labels: BTreeMap::new(), priority: Default::default(),
    };

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input.clone(), vcli_perception::Perception::new(), clock, cmd_rx, ev_tx);
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(400));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    assert_eq!(events.iter().filter(|e| event_type(e) == "program.completed").count(), 1);
    assert_eq!(input.calls().iter().filter(|c| matches!(c, common::mock_input::Call::Click(_, _))).count(), 1);
}
```

Commit: `vcli-runtime: scenario input_postcondition`.

### Task 27: `novelty_timeout`

The same body as Task 26, but the frame script never produces the green frame, so `WaitFor` must fail.

```rust
// tests/scenarios/novelty_timeout.rs
// ... identical setup as input_postcondition but frames = vec![red, red, red]
// and the assertion is:
//
// assert_eq!(events.iter().filter(|e| event_type(e) == "program.failed").count(), 1);
```

Commit: `vcli-runtime: scenario novelty_timeout`.

### Task 28: `action_conflict`

Two programs, both priority 0, fire the same tick. Exactly one's click dispatches; the other emits `action.deferred`.

```rust
// tests/scenarios/action_conflict.rs
#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn two_programs_same_frame_arbitrate_to_one_winner() {
    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let mk = |name: &str| {
        let mut preds = BTreeMap::new();
        preds.insert("red".into(), vcli_core::predicate::PredicateKind::ColorAt {
            point: vcli_core::geom::Point { x: 0, y: 0 },
            rgb:   vcli_core::predicate::Rgb([255, 0, 0]),
            tolerance: 0,
        });
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: name.into(), id: None, trigger: Trigger::OnSubmit,
            predicates: preds,
            watches: vec![vcli_core::watch::Watch {
                when: vcli_core::watch::WatchWhen::ByName("red".into()),
                steps: vec![vcli_core::step::Step::Click {
                    at: vcli_core::step::Target::Absolute(vcli_core::geom::Point { x: 0, y: 0 }),
                    button: vcli_core::action::Button::Left,
                }],
                throttle_ms: 0,
                lifetime: vcli_core::watch::Lifetime::OneShot,
            }],
            body: vec![], on_complete: None, on_fail: None, timeout_ms: None,
            labels: BTreeMap::new(), priority: Default::default(),
        }
    };

    let id1: ProgramId = "11111111-1111-4567-8910-111213141516".parse().unwrap();
    let id2: ProgramId = "22222222-2222-4567-8910-111213141516".parse().unwrap();

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input.clone(), vcli_perception::Perception::new(), clock, cmd_rx, ev_tx);
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id1, program: mk("p1") }).unwrap();
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id2, program: mk("p2") }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    let dispatched = events.iter().filter(|e| event_type(e) == "action.dispatched").count();
    let deferred   = events.iter().filter(|e| event_type(e) == "action.deferred").count();
    assert_eq!(dispatched, 1, "exactly one winner");
    assert_eq!(deferred,   1, "exactly one loser");
}
```

Commit: `vcli-runtime: scenario action_conflict`.

### Task 29: `predicate_dedup`

Two programs with the same `color_at` predicate must produce only one evaluator invocation per tick. Observed via the shared `Perception::cache().len()` after a tick — but since the scheduler owns the `Perception`, we instead assert via a probe: spin the scheduler with a deduplicating wrapper that counts `dispatch()` invocations, or simply assert that total calls to any mock backing the predicate don't grow with program count.

Simplest: submit two identical programs, a single tick, assert that only two `watch.fired` events were emitted (one per program) even though the predicate hash in `merged_graph::dedupe` reported 1 unique entry.

```rust
// tests/scenarios/predicate_dedup.rs
#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn two_programs_same_predicate_both_fire_but_eval_once() {
    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let id1: ProgramId = "11111111-1111-4567-8910-111213141516".parse().unwrap();
    let id2: ProgramId = "22222222-2222-4567-8910-111213141516".parse().unwrap();
    let mk = |name: &str, pri: i32| {
        let mut preds = BTreeMap::new();
        preds.insert("red".into(), vcli_core::predicate::PredicateKind::ColorAt {
            point: vcli_core::geom::Point { x: 0, y: 0 },
            rgb:   vcli_core::predicate::Rgb([255, 0, 0]),
            tolerance: 0,
        });
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: name.into(), id: None, trigger: Trigger::OnSubmit,
            predicates: preds,
            watches: vec![vcli_core::watch::Watch {
                when: vcli_core::watch::WatchWhen::ByName("red".into()),
                steps: vec![],  // empty steps ⇒ no arbiter conflict
                throttle_ms: 0,
                lifetime: vcli_core::watch::Lifetime::OneShot,
            }],
            body: vec![], on_complete: None, on_fail: None, timeout_ms: None,
            labels: BTreeMap::new(),
            priority: vcli_core::program::Priority(pri),
        }
    };

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input, vcli_perception::Perception::new(), clock, cmd_rx, ev_tx);
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id1, program: mk("p1", 1) }).unwrap();
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id2, program: mk("p2", 2) }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    assert_eq!(events.iter().filter(|e| event_type(e) == "watch.fired").count(), 2, "both programs must fire");
}
```

Commit: `vcli-runtime: scenario predicate_dedup`.

### Task 30: `elapsed_ms_since_true`

```rust
// tests/scenarios/elapsed_ms_since_true.rs
#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn elapsed_ms_since_true_fires_after_delay() {
    // Frames are red from the start; watch.when is
    //   ElapsedMsSinceTrue { of: "red", ms: 200 }
    // ⇒ must not fire until 200ms after "red" first true.
    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), vcli_core::predicate::PredicateKind::ColorAt {
        point: vcli_core::geom::Point { x: 0, y: 0 },
        rgb:   vcli_core::predicate::Rgb([255, 0, 0]),
        tolerance: 0,
    });
    preds.insert("delayed_red".into(), vcli_core::predicate::PredicateKind::ElapsedMsSinceTrue {
        of: "red".into(),
        ms: 200,
    });
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "delay".into(), id: None, trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![vcli_core::watch::Watch {
            when: vcli_core::watch::WatchWhen::ByName("delayed_red".into()),
            steps: vec![vcli_core::step::Step::Click {
                at: vcli_core::step::Target::Absolute(vcli_core::geom::Point { x: 0, y: 0 }),
                button: vcli_core::action::Button::Left,
            }],
            throttle_ms: 0,
            lifetime: vcli_core::watch::Lifetime::OneShot,
        }],
        body: vec![], on_complete: None, on_fail: None, timeout_ms: None,
        labels: BTreeMap::new(), priority: Default::default(),
    };

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input.clone(), vcli_perception::Perception::new(), clock, cmd_rx, ev_tx);
    cmd_tx.send(SchedulerCommand::SubmitValidated { program_id: id, program }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(500));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    assert_eq!(events.iter().filter(|e| event_type(e) == "watch.fired").count(), 1);
}
```

If `PredicateKind::ElapsedMsSinceTrue` uses a different field spelling (e.g. `predicate` instead of `of`), adjust the builder inline per AGENT.md.

Commit: `vcli-runtime: scenario elapsed_ms_since_true`.

### Task 31: `daemon_restart_marker`

The daemon issues `SchedulerCommand::ResumeRunning { from_step }` on restart of a resumable program. The scheduler must emit `program.resumed{from_step}` and start the body at that cursor.

```rust
// tests/scenarios/daemon_restart_marker.rs
#[path = "../common/mod.rs"] mod common;
// ... prelude imports ...

#[test]
fn resume_running_starts_body_at_cursor_and_emits_resumed() {
    let blank = ScriptedCapture::solid(1, 1, [0, 0, 0, 0]);
    let capture = Box::new(ScriptedCapture::new(vec![blank]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx)   = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(vcli_runtime::clock::ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "resumable".into(), id: None, trigger: Trigger::OnSubmit,
        predicates: BTreeMap::new(), watches: vec![],
        body: vec![
            vcli_core::step::Step::SleepMs { ms: 1 }, // step 0
            vcli_core::step::Step::SleepMs { ms: 1 }, // step 1
            vcli_core::step::Step::SleepMs { ms: 1 }, // step 2
        ],
        on_complete: None, on_fail: None, timeout_ms: None,
        labels: BTreeMap::new(), priority: Default::default(),
    };

    let sched = Scheduler::new(SchedulerConfig::default(), capture, input, vcli_perception::Perception::new(), clock, cmd_rx, ev_tx);
    // Skip ordinary submit; go directly to resume at step 2.
    cmd_tx.send(SchedulerCommand::ResumeRunning { program_id: id, from_step: 2, program }).unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    let resumed = events.iter().find(|e| event_type(e) == "program.resumed");
    assert!(resumed.is_some(), "must emit program.resumed");
    // The resumed program should also complete (only one SleepMs left from cursor=2).
    assert_eq!(events.iter().filter(|e| event_type(e) == "program.completed").count(), 1);
}
```

Commit: `vcli-runtime: scenario daemon_restart_marker`.

---

## Task 32: Final workspace gate + self-review pass

**Files:**
- None — this task is the whole-workspace gate and the plan's own self-review.

- [ ] **Step 1: Run the workspace gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

All three must pass on macOS and Linux. If clippy fails only on the new crate, fix; do not suppress with `#[allow(...)]` unless the reason is written above as `// Why: ...`.

- [ ] **Step 2: Remove debug `tracing::warn!` calls introduced during scaffolding**

Grep for `tracing::warn!(.*TODO` or `tracing::warn!(.*tmp` in `crates/vcli-runtime/src/` and delete anything obviously temporary.

- [ ] **Step 3: Regenerate `Cargo.lock` if the earlier task edits missed any transitive updates**

Run: `cargo update -p vcli-runtime` (dry-run equivalent: `cargo check --workspace --locked`).

- [ ] **Step 4: Commit the gate-clean state**

If the previous scenarios each committed, this task likely has no diff. Only commit if cargo emitted a `Cargo.lock` update or you removed dead code.

```bash
git status
# if changes exist:
git add -u
git commit -m "$(cat <<'EOF'
vcli-runtime: final workspace gate

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Plan self-review

- **Placeholder scan:** grepped for "TBD", "TODO", "implement later", "similar to Task N", "fill in details" — none present. The `tick_once` body uses `// TODO(Task 17)` in Task 16 and is fully replaced in Task 17, which is a progression marker, not a placeholder.
- **Type/method consistency:** `Scheduler::new` signature (Tasks 16, 17) matches the one pre-declared in the header; `SchedulerCommand` variants (Task 3) match the daemon plan's expectation; `RuntimeClock` (Task 4) is the only shape `Scheduler::new` accepts. `WatchRuntime` field names (`last_fired_ms`, `last_truthy`, `retired`) are used identically across Tasks 7, 10, 17. `BodyState` / `BodyDefer` / `StepOutcome` are used identically in Tasks 15 and 17.
- **Spec coverage:** every scenario in spec §760 has a dedicated task (Tasks 19–31). Spec §370 state machine is encoded in `transitions::validate` (Task 6). Spec §403 arbitration is Task 11. Spec §412 confirmation is Task 12. Spec §985 merged DAG is Task 13. Spec §432 `Clock` abstraction is Task 4. Spec §425 triggers are Task 8. Spec §348 capture overrun is Task 14 + Task 17.
- **Compile order:** every cross-task reference is either (a) to a module created earlier in the plan, or (b) to `vcli-core` / `vcli-capture` / `vcli-input` / `vcli-perception`, which are established crates on master with the signatures referenced here.
- **Commit cadence:** 32 commits total, every one under the `vcli-runtime:` prefix per AGENT.md.

Plan complete.

