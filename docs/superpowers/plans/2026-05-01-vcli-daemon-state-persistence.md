# vcli-daemon state-persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist scheduler lifecycle events into SQLite so `vcli list` and `vcli status` reflect the daemon's actual in-memory program state.

**Architecture:** Keep the fix inside `vcli-daemon::persist`, the existing event-pump boundary where scheduler events are already written before IPC broadcast. The pump will continue appending program events, and will additionally update `programs.state`, `programs.started_at`, `programs.finished_at`, and `programs.last_error_*` for lifecycle/terminal events. This avoids a runtime-to-store dependency and preserves the current channel boundary.

**Tech Stack:** Rust 2021, existing `vcli-core`, `vcli-daemon`, and `vcli-store` APIs. No new dependencies.

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md` §Persistence and Decision B6.

---

## File structure

```
crates/vcli-daemon/src/persist.rs      # modify event-pump persistence and unit coverage
```

No schema change is required: the `programs` table already contains `state`, `started_at`, `finished_at`, `last_error_code`, and `last_error_msg`.

## Task 1: persist lifecycle state before broadcast

**Files:**
- Modify: `crates/vcli-daemon/src/persist.rs`

- [ ] **Step 1: Write failing tests**

Add tests in `crates/vcli-daemon/src/persist.rs` proving the event pump updates the program row before broadcasting:

```rust
#[tokio::test]
async fn state_changed_updates_program_row_before_broadcast() {
    let d = tempdir().unwrap();
    let (mut store, _) = Store::open(d.path()).unwrap();
    let pid = ProgramId::new();
    store.insert_program(&NewProgram {
        id: pid,
        name: "x",
        source_json: "{}",
        state: vcli_core::ProgramState::Pending,
        submitted_at: 0,
        labels_json: "{}",
    }).unwrap();
    let store = Arc::new(Mutex::new(store));
    let (sched_tx, sched_rx) = unbounded::<Event>();
    let (bcast_tx, mut bcast_rx) = broadcast::channel::<Event>(16);
    let pump = spawn_event_pump(store.clone(), sched_rx, bcast_tx);

    let ev = Event {
        at: 11,
        data: EventData::ProgramStateChanged {
            program_id: pid,
            from: vcli_core::ProgramState::Pending,
            to: vcli_core::ProgramState::Waiting,
            reason: "submitted".into(),
        },
    };
    sched_tx.send(ev).unwrap();
    bcast_rx.recv().await.unwrap();
    assert_eq!(store.lock().unwrap().get_program(pid).unwrap().state, vcli_core::ProgramState::Waiting);

    drop(sched_tx);
    pump.await.unwrap();
}

#[tokio::test]
async fn program_failed_persists_terminal_state_and_last_error_before_broadcast() {
    let d = tempdir().unwrap();
    let (mut store, _) = Store::open(d.path()).unwrap();
    let pid = ProgramId::new();
    store.insert_program(&NewProgram {
        id: pid,
        name: "x",
        source_json: "{}",
        state: vcli_core::ProgramState::Running,
        submitted_at: 0,
        labels_json: "{}",
    }).unwrap();
    let store = Arc::new(Mutex::new(store));
    let (sched_tx, sched_rx) = unbounded::<Event>();
    let (bcast_tx, mut bcast_rx) = broadcast::channel::<Event>(16);
    let pump = spawn_event_pump(store.clone(), sched_rx, bcast_tx);

    let ev = Event {
        at: 12,
        data: EventData::ProgramFailed {
            program_id: pid,
            reason: "assert_failed".into(),
            step: Some("body[0]".into()),
            emit: None,
        },
    };
    sched_tx.send(ev).unwrap();
    bcast_rx.recv().await.unwrap();
    let row = store.lock().unwrap().get_program(pid).unwrap();
    assert_eq!(row.state, vcli_core::ProgramState::Failed);
    assert_eq!(row.last_error_code.as_deref(), Some("assert_failed"));
    assert_eq!(row.last_error_msg.as_deref(), Some("body[0]"));

    drop(sched_tx);
    pump.await.unwrap();
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p vcli-daemon --lib event_pump_persists
```

Expected: both tests fail because `spawn_event_pump` currently appends events but does not update the program row.

- [ ] **Step 3: Implement minimal persistence helper**

Add a helper in `persist.rs` and call it while the store mutex is held, before broadcasting:

```rust
fn persist_program_event(store: &mut Store, pid: ProgramId, ev: &Event) {
    if let Err(e) = store.append_event(pid, ev) {
        error!(error = %e, "failed to append event");
    }
    match &ev.data {
        EventData::ProgramStateChanged { to, .. } => {
            if let Err(e) = store.update_state(pid, *to, ev.at) {
                error!(error = %e, "failed to update program state");
            }
        }
        EventData::ProgramCompleted { .. } => {
            if let Err(e) = store.update_state(pid, vcli_core::ProgramState::Completed, ev.at) {
                error!(error = %e, "failed to mark program completed");
            }
        }
        EventData::ProgramFailed { reason, step, .. } => {
            if let Err(e) = store.update_state(pid, vcli_core::ProgramState::Failed, ev.at) {
                error!(error = %e, "failed to mark program failed");
            }
            let msg = step.as_deref().unwrap_or(reason);
            if let Err(e) = store.set_last_error(pid, reason, msg) {
                error!(error = %e, "failed to persist last error");
            }
        }
        EventData::ProgramResumed { from_step, .. } => {
            if let Err(e) = store.update_state(pid, vcli_core::ProgramState::Running, ev.at) {
                error!(error = %e, "failed to mark program resumed");
            }
            if let Err(e) = store.set_body_cursor(pid, *from_step) {
                error!(error = %e, "failed to persist resume cursor");
            }
        }
        _ => {}
    }
}
```

- [ ] **Step 4: Run focused tests**

Run:

```bash
cargo test -p vcli-daemon --lib event_pump_persists
```

Expected: both tests pass.

- [ ] **Step 5: Run required gates**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/plans/2026-05-01-vcli-daemon-state-persistence.md crates/vcli-daemon/src/persist.rs
git commit -m "vcli-daemon: persist scheduler lifecycle state

Co-Authored-By: Codex GPT-5 <noreply@openai.com>"
```
