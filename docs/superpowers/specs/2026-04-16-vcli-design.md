---
title: vcli — design
status: proposed
date: 2026-04-16
author: @blaksmatic
---

# vcli — design

## Summary

**vcli** (Vision CLI) is a local, persistent screen-control runtime that AI agents (and humans) command through declarative JSON programs. A daemon loads each program and executes it reactively against live screen state — no agent-in-the-loop on the hot path. Think of it as a small operating system for screen-scoped programs, driven by a Unix-philosophy CLI.

The thesis is the robotics hierarchy: a slow planner (LLM, out of scope for the daemon) emits a reactive program; a fast runtime executes it at frame rate (10fps target) and only calls back to the planner on completion, failure, or explicit novelty. One LLM call per task, not per frame.

This is a personal open-source lab project. Goals are craft, learning, and a portfolio-worthy artifact. Local-first, no data leaves the machine.

## Goals

- Prove the reactive-runtime thesis end-to-end on macOS.
- Ship a canonical demo (YouTube ad skipper) that a non-technical person recognizes as "a thing they'd actually use."
- Keep the daemon → CLI → planner boundaries clean enough that each piece can evolve independently.
- Design for macOS + Windows (traits from day 1); ship macOS first.
- Predictable frame budget at 10fps with headroom for Tier-3 perception later.

## Non-goals

- Cross-machine orchestration. Local-first is the whole point.
- Proprietary hosted model runtimes. Everything runs on the user's hardware.
- Browser-extension-level DOM interactivity. We operate at the screen/input layer intentionally.
- Mobile / touch device control.
- LLM planner integration in v0. The daemon accepts JSON from any source; the planner is a separate layer built later.

## v0 scope

**In:**

- Rust daemon + thin Rust CLI client
- Unix-socket IPC with framed JSON
- macOS capture (ScreenCaptureKit via `core-graphics`) + input (`core-graphics` events or `enigo`) backends
- `Capture` and `Input` traits with mock backends for testing and stub slots for Windows
- JSON DSL: predicates, regions, watches, body, triggers, completion events
- Tiered perception: Tier 1 (`color_at`, `pixel_diff`, logical, `elapsed_ms_since_true`) + Tier 2 (`template`)
- 10fps scheduler with shared capture, shared predicate cache, action arbitration
- Program lifecycle: pending → waiting → running → blocked → completed | failed | cancelled
- SQLite-backed program store, content-addressed asset store, in-memory ring-buffer trace
- `vcli resume` for post-restart continuation of resumable programs with body-cursor checkpointing
- Unit tests, scenario tests (property assertions + determinism-gated golden trace diffs), DSL fuzz, real-Safari+YouTube E2E demo (gated behind `--features e2e`)

**Out (deferred to later phases):**

- OCR (v0.2), VLM (v0.3)
- Windows backend (v0.4)
- Planner integration (v0.5; separate repo)
- Games / DirectInput support
- Multi-display capture
- Remote daemon with auth

## Architecture

Single Cargo workspace. Dependency arrows point downward — no cycles.

```
vcli/
├── Cargo.toml                      # workspace
├── crates/
│   ├── vcli-core/                  # shared types. Zero runtime deps.
│   ├── vcli-dsl/                   # parse + validate JSON programs
│   ├── vcli-capture/               # Capture trait + macOS impl
│   ├── vcli-input/                 # Input trait + macOS impl
│   ├── vcli-perception/            # evaluators + PerceptionCache
│   ├── vcli-runtime/               # scheduler / tick loop / arbitration
│   ├── vcli-store/                 # SQLite + AssetStore + TraceBuffer
│   ├── vcli-ipc/                   # framed JSON over socket
│   ├── vcli-daemon/                # binary: wires everything; tokio + signals
│   └── vcli-cli/                   # binary: thin client
├── assets/fixtures/                # test fixtures (canned frames, template PNGs)
├── docs/
└── xtask/                          # codegen, integration helpers
```

### Crate responsibilities

- **`vcli-core`** — `Program`, `Predicate`, `Action`, `Step`, `Region`, `ProgramId`, `Frame`, `Match`, `ProgramState`, event types, error types. Pure `serde` derives. No `tokio`, no runtime deps. Unit tests stay instant.
- **`vcli-dsl`** — JSON → validated `Program`. Validation catches unknown predicate references, cycles in `relative_to`, unknown action/predicate kinds, missing assets (before they reach the scheduler), malformed expressions, unsupported DSL majors.
- **`vcli-capture`** — `trait Capture { fn grab(&mut self) -> Result<Frame>; }`. macOS impl via `core-graphics` (ScreenCaptureKit). Mock impl (`CannedSequenceCapture`) returns pre-loaded PNGs per call.
- **`vcli-input`** — `trait Input { fn dispatch(&mut self, action: InputAction) -> Result<()>; }`. macOS impl via `core-graphics` events (or `enigo` — evaluated at impl time). Mock impl records calls.
- **`vcli-perception`** — `trait PredicateEvaluator` + v0 evaluators + `PerceptionCache`. Image decode, template matching (imageproc NCC), color + pixel-diff, logical composition.
- **`vcli-runtime`** — the scheduler. Sync tick loop driven by a timer. Accepts `impl Capture` + `impl Input` + `impl Clock`. All interesting logic lives here.
- **`vcli-store`** — SQLite schema + migrations + asset CAS + trace ring buffers.
- **`vcli-ipc`** — frame codec, `Request` / `Response` / `Event` types, error payloads.
- **`vcli-daemon`** — bin. Tokio reactor, signals, launchd/systemd friendliness. Owns the runtime handle. Bridges socket messages into the runtime via channels.
- **`vcli-cli`** — bin. Clap command parser, IPC client, pretty + `--json` output modes.

### Threading model

- **Single dedicated thread for the tick loop.** Deterministic state transitions.
- **`rayon` pool for parallel Tier-1/Tier-2 evaluation** of unique predicates within a tick.
- **`tokio` runtime in the daemon** for IPC, signals, and bridging — never contaminates the scheduler's logic.
- **Dedicated worker threads for Tier-3 evaluators** (OCR, VLM — v0.2+). Bounded channels. Tick loop reads last cached result; never awaits Tier-3 inline.

## DSL

### Program shape

```jsonc
{
  "version": "0.1",
  "name": "yt-ad-skipper",
  "id": null,                                 // optional; daemon assigns UUID if null

  "trigger": { "kind": "on_submit" },         // on_submit | on_predicate | on_schedule | manual

  "predicates": { "<name>": { /* Predicate */ } },

  "watches": [ /* Watch */ ],                 // reactive, evaluated every tick while running
  "body":    [ /* Step */ ],                  // sequential, runs once top-to-bottom

  "on_complete": { "emit": "<event_name>" },
  "on_fail":     { "emit": "<event_name>" },

  "timeout_ms": 300000,                       // null = no limit
  "labels": { "owner": "me", "kind": "demo" }
}
```

### Predicate kinds (v0)

```jsonc
{ "kind": "template",   "image": "sha256:…" | "assets/skip.png",
                        "confidence": 0.9, "region": Region, "throttle_ms": 200 }

{ "kind": "color_at",   "point": {"x": 100, "y": 200}, "rgb": [255,0,0], "tolerance": 15 }

{ "kind": "pixel_diff", "region": Region, "baseline": "sha256:…", "threshold": 0.05 }

{ "kind": "all_of",     "of": ["pred_a", "pred_b"] }
{ "kind": "any_of",     "of": ["pred_a", "pred_b"] }
{ "kind": "not",        "of": "pred_a" }

{ "kind": "elapsed_ms_since_true", "predicate": "pred_a", "ms": 1000 }
```

A predicate evaluation returns `PredicateResult { truthy: bool, match: Option<MatchData>, at: Timestamp }`. `match` is populated for kinds that produce locations (`template` → bounding box + center + confidence). Logical kinds produce no match.

### Region kinds

```jsonc
{ "kind": "absolute", "box": { "x": 0, "y": 0, "w": 1920, "h": 1080 } }

{ "kind": "window", "app": "Safari", "title_contains": "YouTube" }

{ "kind": "relative_to", "predicate": "on_cart_page", "anchor": "match",
  "offset": { "x": 0, "y": 40 }, "size": { "w": 300, "h": 120 } }
```

`window` regions resolve via the macOS Accessibility API each tick, with a short-TTL (~500ms) cache to avoid hammering AX. Missing windows produce a non-truthy predicate — not an error.

`relative_to` regions resolve after their referenced predicate evaluates (topological sort within a tick). If the reference isn't truthy, the dependent is not truthy either.

### Watch (reactive rule)

```jsonc
{
  "when": "skip_visible",                      // predicate name or inline object
  "do":   [ /* Step */ ],
  "mode": "rising_edge",                       // rising_edge | while_true
  "throttle_ms": 500,
  "lifetime": { "kind": "persistent" },        // persistent | one_shot |
                                               // until_predicate(name) | timeout_ms(N)
  "on_fire": { "emit": "<event_name>" }        // optional custom event after successful firing
}
```

Watches are tracked per-(program, watch) and become eligible according to `mode`:

- `rising_edge` (default) — fires when `when` transitions false→true, respecting `throttle_ms`.
- `while_true` — becomes eligible whenever `when` is truthy and `throttle_ms` has elapsed since the last completed or deferred attempt. Use for sticky UI affordances where a dropped click should retry without requiring the screen to go false first.

A watch firing is counted only after its `do` sequence completes successfully. `one_shot` fires exactly once; `until_predicate` runs persistently until the named predicate is truthy, then is removed. On successful firing the daemon emits `watch.fired` and, if present, the custom event named by `on_fire.emit`.

### Step (body and `watch.do` share the vocabulary)

```jsonc
// Input actions
{ "kind": "click",  "at": "$skip_visible.match.center", "button": "left" }
{ "kind": "type",   "text": "hello world" }
{ "kind": "key",    "combo": ["cmd", "s"] }
{ "kind": "scroll", "at": "$target.match.center", "dy": -400 }
{ "kind": "move",   "at": "$foo.match.top_left" }

// Control flow (body only)
{ "kind": "wait_for",  "predicate": "on_cart_page", "timeout_ms": 5000,
                       "on_timeout": "fail" }           // fail | continue | retry
{ "kind": "assert",    "predicate": "item_row_visible", "on_fail": "fail" }
{ "kind": "sleep_ms",  "ms": 250 }

// Reserved — schema-valid, not implemented in v0
{ "kind": "subprogram", "ref": "<program_id>" }
```

### Input postconditions

Any input action may optionally require a visual postcondition:

```jsonc
"postcondition": { "predicate": "skip_gone", "within_ms": 1500, "on_timeout": "novelty" }
// on_timeout = fail | continue | novelty
```

`Input::dispatch` success only means the OS accepted the event. If `postcondition` is present, the step is not considered successful until the predicate becomes truthy within `within_ms`.

- `fail` — transition to `program.failed`
- `continue` — continue execution without a visual success guarantee
- `novelty` — emit `program.novelty` and then fail the program with reason `novelty_timeout`

### Expression language

Dotted paths only. No arithmetic, no conditionals in v0.

```
$<predicate_name>.match.center        // Point
$<predicate_name>.match.top_left      // Point
$<predicate_name>.match.box           // Rect
$<predicate_name>.match.confidence    // f32
```

References to predicates without a match produce a step error (`program.failed` with a clear diagnostic). No silent null propagation.

### Validation

`vcli-dsl` rejects before the scheduler ever sees a program:

- Unknown predicate name referenced from watch, step, step postcondition, region, or expression
- Cycle in `relative_to` predicate graph
- Unknown action or predicate `kind`
- Missing `image` asset (not in asset store and not a readable file path)
- Malformed expression (unknown accessor, unknown predicate)
- `version` major the daemon doesn't understand

Errors include JSON path info: `{ "code": "invalid_program", "message": "unknown predicate 'foo'", "path": "watches[0].when" }`.

### Full example — YT ad skipper

```json
{
  "version": "0.1",
  "name": "yt-ad-skipper",
  "trigger": { "kind": "on_submit" },
  "predicates": {
    "skip_visible": {
      "kind": "template",
      "image": "assets/yt_skip.png",
      "confidence": 0.9,
      "region": { "kind": "window", "app": "Safari", "title_contains": "YouTube" },
      "throttle_ms": 200
    },
    "skip_gone": { "kind": "not", "of": "skip_visible" }
  },
  "watches": [
    {
      "when": "skip_visible",
      "mode": "while_true",
      "do":   [{
        "kind": "click",
        "at": "$skip_visible.match.center",
        "postcondition": { "predicate": "skip_gone", "within_ms": 1500, "on_timeout": "novelty" }
      }],
      "throttle_ms": 500,
      "lifetime": { "kind": "persistent" },
      "on_fire": { "emit": "ad_skipped" }
    }
  ],
  "body": []
}
```

### Full example — sequential buy-the-book

```json
{
  "version": "0.1",
  "name": "buy-the-book",
  "trigger": { "kind": "on_submit" },
  "predicates": {
    "cart_icon":         { "kind": "template", "image": "assets/cart.png",        "confidence": 0.9 },
    "on_cart_page":      { "kind": "template", "image": "assets/cart_header.png", "confidence": 0.9 },
    "item_row_visible":  { "kind": "template", "image": "assets/book_thumb.png",
                           "region": { "kind": "relative_to", "predicate": "on_cart_page" },
                           "confidence": 0.85 }
  },
  "body": [
    { "kind": "click",    "at": "$cart_icon.match.center" },
    { "kind": "wait_for", "predicate": "on_cart_page",    "timeout_ms": 5000, "on_timeout": "fail" },
    { "kind": "assert",   "predicate": "item_row_visible", "on_fail": "fail" }
  ],
  "watches": [],
  "on_complete": { "emit": "purchase_verified" }
}
```

### Design properties

- **One vocabulary, two contexts.** `watch.do` and `body` use the same `Step` type. Only `body` gets `wait_for` / `assert` / `sleep_ms`.
- **Predicates are first-class and named.** Enables structural dedup across programs.
- **Progress and completion are distinct.** Pure-watch programs emit progress via `watch.on_fire.emit`; `on_complete` remains terminal-only.
- **No program-level variables in v0.** Stateful semantics are encoded via predicates (e.g., `elapsed_ms_since_true`). Avoids imperative-in-a-reactive-language confusion. Variables can be added later without breaking existing programs.
- **No body conditionals in v0.** `wait_for` + `assert` + watches cover the v0 demo programs. `if`/`else` added only when a concrete program needs them.
- **Extension slots reserved** in the schema (e.g., `subprogram`) so later versions add features without schema churn.

## Runtime & scheduler

### Tick loop

```
loop {
  tick_start = now()
  frame      = capture.grab()                     // one capture, shared across programs

  active_preds = scheduler.dedup_predicates(
                   running_programs + waiting_predicate_trigger_programs
                 )

  // Tier 1: evaluate cheap predicates every tick
  // Tier 2: evaluate throttled predicates if last_eval + throttle_ms <= now
  // Tier 3 (post-v0): never evaluated inline — cache read only
  perception_results = perception.evaluate(&frame, &active_preds, cache.as_mut())

  for program in waiting_predicate_trigger_programs {
    program.maybe_fire_trigger(&perception_results, &clock)
  }

  for program in running_programs {
    program.advance(&perception_results, &clock)   // emits zero or more pending actions
  }

  chosen_actions = action_arbiter.resolve(all_pending_actions)

  for action in chosen_actions { input.dispatch(action) }   // synchronous with confirmation

  event_bus.drain_and_publish()
  trace.append(tick_record)

  sleep_until(tick_start + TICK_PERIOD)           // TICK_PERIOD = 100ms @ 10fps
}
```

### Shared capture

Exactly one `capture.grab()` per tick. Frame is an `Arc<Frame>`, no copies. If capture exceeds the tick budget, the scheduler **skips ticks** rather than stacking them — emits `tick.frame_skipped { reason: "capture_overrun" }` and resumes on the next timer.

### Predicate cache

```rust
struct PerceptionCache {
    entries: HashMap<PredicateHash, CacheEntry>,
}
struct CacheEntry {
    last_result:  PredicateResult,
    last_eval_at: Instant,
    refcount:     usize,
}
```

- **Key = content hash** of the canonical-serialized, fully-resolved predicate subgraph (names stripped; asset paths resolved to content hashes first). `relative_to` and logical predicates hash over their canonicalized dependencies, not just their local node.
- **Program-local temporal predicates are never cross-program deduped.** `elapsed_ms_since_true` layers program-local transition timing on top of a shared child predicate result.
- **Refcount** on program entering a predicate-evaluated state (`running`, plus `waiting` for `on_predicate` triggers); decrement on leaving. Evicted when zero on the next tick.
- **Throttle check lives in the cache**, not in programs — consistent semantics regardless of reference count.

### Program state machine

```
                    submit()
                       │
                       ▼
                   pending
                       │
                    daemon ready
                       ▼
                   waiting ──── trigger fires ────▶ running
                                                      │
                                                      ├── body step errors / assert fails → failed
                                                      ├── wait_for timeout (on_timeout=fail) → failed
                                                      ├── input postcondition timeout (on_timeout=novelty) → failed
                                                      ├── program-level timeout_ms         → failed
                                                      ├── cancel request                   → cancelled
                                                      ├── body complete (non-empty body)   → completed
                                                      └── blocked (future)                 → blocked
                                                              │
                                                           unblock
                                                              ▼
                                                           running
```

`blocked` is reserved for v0 but no feature transitions into it. Allows later features (external-event awaiting) without schema migration.

**Pure-watches programs.** A program with an empty `body` has no natural completion — it stays `running` until cancelled, times out via program-level `timeout_ms`, or an `until_predicate`-lifetimed watch's guard becomes truthy and that was the only remaining watch (no remaining active watches = completed). Pure-watch programs still emit progress through `watch.fired` and optional `watch.on_fire.emit` custom events. The YT ad skipper has `timeout_ms: null` and a `persistent` watch, so it runs until cancelled — that's the intended behavior.

**`on_complete` / `on_fail` are event emitters, not transition triggers.** When a program transitions to `completed` or `failed` for any reason above, the daemon fires `program.completed` / `program.failed` system events, and *additionally* emits the optional custom event name specified in `on_complete.emit` / `on_fail.emit`. `watch.on_fire.emit` is the non-terminal counterpart for long-running reactive programs.

Transitions are atomic within a tick and emit `program.state_changed { from, to, reason }`.

### Action arbitration

At step 6 of the tick loop the arbiter resolves pending actions from all programs:

1. **Mouse exclusivity.** At most one mouse action (move, click, scroll) per tick. Highest-priority pending action wins; ties broken by earliest submission time, then `program_id` for determinism.
2. **Keyboard exclusivity.** At most one keyboard action (type, key) per tick. Same resolution.
3. **Mouse + keyboard can coexist** in the same tick.
4. **No stale re-queue.** Arbitration losers are not carried as pending input objects across ticks. Body steps remain at their current cursor and re-offer the action on the next tick. Watch firings do not count as fired; `rising_edge` watches wait for the next false→true transition, while `while_true` watches may retry after `throttle_ms`. Losers emit `action.deferred { program_id, reason: "conflict_with": <other_program_id> }` to the trace.

### Action confirmation

`Input::dispatch` returns `Result<(), InputError>` synchronously (microseconds for OS-level confirmation — not visual confirmation). If an input step has no `postcondition`, dispatch confirmation is success. If it has a `postcondition`, body steps advance and watch-triggered actions are counted as fired only after the postcondition becomes truthy within `within_ms`. `on_timeout=fail` transitions to `failed`; `on_timeout=continue` advances without a visual success guarantee; `on_timeout=novelty` emits `program.novelty` and then fails the program with reason `novelty_timeout`. Body cursor is written to SQLite only after a step is resolved.

### Watch lifetimes

| Lifetime | Behavior |
|---|---|
| `one_shot` | Fires once on its first eligible firing, then is removed for this program run. |
| `persistent` | Remains active until program end; firing eligibility is determined by `mode` and `throttle_ms`. |
| `until_predicate(name)` | Persistent until the named predicate becomes truthy; then removed. |
| `timeout_ms(N)` | Persistent until N ms after program started; then removed. |

### Trigger evaluation (v0)

- `on_submit` — fires immediately when daemon is ready.
- `on_predicate` — program stays `waiting`; runtime evaluates the trigger predicate (and its dependencies) every tick until truthy, then transitions to `running`.
- `on_schedule` — cron-ish expression, checked once per tick (v0.1 deferral).
- `manual` — stays in `waiting` until `vcli start <id>`.

### Clock abstraction

All time reads go through a `Clock` trait. Production impl uses `Instant::now()`; `TestClock` fast-forwards deterministically. Essential for testing lifetimes, throttles, and timeouts without `thread::sleep`.

## Perception pipeline

### Trait

```rust
pub trait PredicateEvaluator: Send + Sync {
    fn kind(&self) -> PredicateKind;
    fn cost_class(&self) -> CostClass;    // Cheap | Medium | Expensive
    fn evaluate(&self, frame: &Frame, predicate: &PredicateSpec,
                cache: &mut PerceptionCache) -> PredicateResult;
}
```

The scheduler orchestrates by cost class and has no knowledge of specific evaluators.

### Tiered policy

| Tier | Cost | When | v0 evaluators |
|------|------|------|---------------|
| **1 — Cheap** | <1ms | Every tick | `color_at`, `pixel_diff`, logical, `elapsed_ms_since_true` |
| **2 — Medium** | 5–30ms | Respecting `throttle_ms` | `template` |
| **3 — Expensive** | 100ms+ | Out-of-band worker; cache-read only (v0.2+) | `ocr`, `vlm` |

Tier 1 runs inline on the tick thread. Tier 2 fans out to `rayon` across unique predicates within the tick. Tier 3 runs on dedicated worker threads with bounded request channels; the tick loop reads the last cached result and issues a new request only when staleness permits.

### Template matching

- `imageproc::template_matching::match_template` with normalized SSE (NCC-ish).
- **Region-scoped by default.** Validator emits a warning at submit if a template has a full-display `absolute` region and `confidence < 0.95`.
- Pyramid search is **not implemented in v0** — added only if full-res blows the tick budget.
- `confidence` is an inclusive threshold.
- **Asset loading:** at submit, `image: "path.png"` is read, hashed, inserted into the asset store, and rewritten in the stored program to `sha256:…`. Decoded `DynamicImage`s are kept in a bounded LRU keyed by hash (not by program), shared across programs.

### Color / pixel diff

- `color_at`: single-pixel read + RGB Euclidean distance within tolerance.
- `pixel_diff`: perceptual hash (dHash) over region + Hamming distance against baseline hash.

### Logical composition

`all_of` / `any_of` / `not` combine dependencies' `.truthy` flags. No fresh cost. Dependencies topologically sorted per tick; cycles rejected at submit.

### `elapsed_ms_since_true`

Per-(program, predicate) state (last transition time) lives on the program's runtime state, not in the shared cache — it's program-local. The referenced child predicate may still be shared through `PerceptionCache`; only the elapsed-time wrapper is local. Microsecond cost.

### Window region resolution (macOS v0)

- macOS Accessibility API (`AXUIElement`) → window geometry.
- Resolved box cached ~500ms per `(app, title_contains)` tuple.
- No matching window → predicate is not truthy (not an error). Programs quietly no-op when target app is absent.

### Caches

1. **`PerceptionCache`** (shared, per predicate hash) — last result + eval time + refcount.
2. **Template asset LRU** (shared, per image hash) — decoded `DynamicImage`s.
3. **Window geometry cache** (short TTL) — per `(app, title_contains)` tuple.

All three instrumented with tick-time histograms emitted in trace records.

### v0 exclusions (extensibility preserved)

- No OCR, no VLM — both slot in behind the existing `PredicateEvaluator` trait without scheduler changes.
- No motion / optical flow.
- No multi-display — single primary display in v0.
- No text-color heuristics.

## IPC protocol & CLI

### Socket

```
$XDG_RUNTIME_DIR/vcli.sock
~/Library/Application Support/vcli/vcli.sock   # fallback
```

Permissions `0600` (owner only). Daemon creates on start; unlinks on shutdown. CLI discovers via the same resolution.

### Wire format

Length-prefixed frames: `u32` big-endian length, then UTF-8 JSON. Binary-safe, debuggable, zero new deps.

### Messages

```jsonc
// Request
{ "id": "<uuid>", "op": "submit"|"list"|"status"|"cancel"|"start"|"resume"|
                        "logs"|"events"|"trace"|"health"|"gc"|"shutdown",
  "params": { /* op-specific */ } }

// Response (non-streaming)
{ "id": "<uuid>", "ok": true,  "result": { /* op-specific */ } }
{ "id": "<uuid>", "ok": false, "error": { "code": "…", "message": "…", "path": "…" } }

// Event (pushed on streaming ops like logs / events)
{ "stream": "events", "type": "program.state_changed",
  "program_id": "…", "data": { "from": "waiting", "to": "running" },
  "at": "2026-04-16T19:42:11.123Z" }
```

### Events (v0)

```
program.submitted            { program_id, name }
program.state_changed        { program_id, from, to, reason }
program.novelty             { program_id, reason, step?, predicate? }
program.completed            { program_id, emit? }
program.failed               { program_id, reason, step?, emit? }
program.resumed              { program_id, from_step }
watch.fired                  { program_id, watch_index, predicate, emit? }
action.dispatched            { program_id, step, target? }
action.deferred              { program_id, step, reason }
tick.frame_skipped           { reason }
capture.permission_missing   { backend }
daemon.started / daemon.stopped
```

Per-tick predicate evaluations are written to the in-memory trace but **not** broadcast on `events` — too chatty. Access via `vcli trace dump`.

### Error codes (v0)

- `invalid_program` — DSL validation failure; accompanied by JSON path
- `unknown_program`
- `bad_state_transition` — e.g., `cancel` on a completed program
- `not_resumable` — resume requested for a program whose semantics cannot be recovered from `body_cursor`
- `permission_denied` — macOS Accessibility or Screen Recording not granted
- `capture_failed`
- `novelty_timeout` — a step postcondition did not become true before its deadline
- `daemon_busy`
- `internal` — logged server-side with correlation id

### CLI surface (v0)

```
vcli submit <program.json>              # → program_id. Resolves assets, validates, submits.
vcli list [--state STATE]               # tabular list of programs
vcli status <program_id>                # detailed status
vcli cancel <program_id>                # running → cancelled; idempotent
vcli start <program_id>                 # fires a manual trigger; waiting → running
vcli resume <program_id> [--from-start] # continues after daemon_restart for resumable programs only

vcli logs <program_id> [--follow]       # streams program-scoped events
vcli events [--follow]                  # streams all events (firehose)
vcli trace dump <program_id> [--save-to file.jsonl]

vcli health                             # version, uptime, tick stats, cache sizes
vcli daemon start                       # fork + detach; idempotent
vcli daemon run                         # foreground; for launchd/systemd
vcli daemon stop                        # graceful
vcli daemon status                      # running?, pid, socket path
vcli gc                                 # asset GC; manual
```

All commands support `--json`. Exit codes: `0` success, `1` generic, `2` validation, `3` not found, `4` daemon not running.

### Auth

**None in v0.** Socket permissions (`0600`, owner-only) are the security boundary — anyone who can connect already has your user account. Documented in the README. Token / TLS auth lands when remote daemon becomes a thing (post-v0).

## Persistence, tracing, errors

### Data layout (macOS v0)

```
~/Library/Application Support/vcli/
├── vcli.db                        # SQLite
├── assets/sha256/ab/cd/abcd…ef.png
├── daemon.pid
└── config.toml                    # optional

~/Library/Logs/vcli/daemon.log     # rotated daily, 7-day retention
```

### SQLite schema (v0)

```sql
CREATE TABLE programs (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    source_json     TEXT NOT NULL,                -- canonical form
    state           TEXT NOT NULL,                -- pending|waiting|running|blocked|
                                                  -- completed|failed|cancelled
    submitted_at    INTEGER NOT NULL,
    started_at      INTEGER,
    finished_at     INTEGER,
    last_error_code TEXT,
    last_error_msg  TEXT,
    labels_json     TEXT NOT NULL DEFAULT '{}',
    body_cursor     INTEGER NOT NULL DEFAULT 0,   -- next body step to execute
    body_entered_at INTEGER                       -- unix ms when body began
);
CREATE INDEX programs_state_idx ON programs(state);

CREATE TABLE program_assets (
    program_id  TEXT NOT NULL REFERENCES programs(id) ON DELETE CASCADE,
    asset_hash  TEXT NOT NULL,
    PRIMARY KEY (program_id, asset_hash)
);
CREATE INDEX program_assets_hash_idx ON program_assets(asset_hash);

CREATE TABLE events (                             -- durable terminal events only
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    program_id  TEXT NOT NULL REFERENCES programs(id) ON DELETE CASCADE,
    type        TEXT NOT NULL,
    data_json   TEXT NOT NULL,
    at          INTEGER NOT NULL
);
CREATE INDEX events_program_idx ON events(program_id);
CREATE INDEX events_at_idx      ON events(at);

CREATE TABLE schema_version (version INTEGER NOT NULL);
INSERT INTO schema_version VALUES (1);
```

- **In the DB:** programs, state, durable terminal events, asset refs, body cursor.
- **Not in the DB:** per-tick traces (in-memory ring), transient events (bus only), frame captures (never stored).

### Asset store (content-addressed)

- `sha256(bytes)` → file at `assets/sha256/<2>/<2>/<hex>.<ext>`.
- At submit, the DSL layer walks the program, reads each `image: "path"` from disk, hashes it, inserts (idempotent), and rewrites the program JSON to `image: "sha256:<hash>"`. Stored `source_json` always references hashes — programs become self-contained and hash-equivalent.
- GC opportunistic: `vcli gc` is explicit; daemon triggers GC on startup if last run was >7 days ago. Never blocks the tick loop.

### Trace buffer (in-memory)

```rust
struct TraceBuffer {
    per_program: DashMap<ProgramId, RingBuffer<TraceRecord>>,   // 10_000 records each
    global:      RingBuffer<TraceRecord>,                        // 100_000 records
}
struct TraceRecord {
    tick:       u64,
    at:         UnixMs,
    program_id: Option<ProgramId>,
    kind:       TraceKind,                                       // predicate_eval, state_change, …
    payload:    serde_json::Value,
}
```

Capped at ~50MB worst-case. `vcli trace dump <id>` emits newline-delimited JSON to stdout; `--save-to file.jsonl` persists on demand.

### Restart semantics

On daemon startup:

1. Read config.
2. Open SQLite, run migrations.
3. Scan assets vs. `program_assets` refs; log orphan count (no auto-GC).
4. **Programs in state `running` transition to `failed` with code `daemon_restart`**, reason `"daemon restarted during execution"`. Planner sees the event cleanly.
5. Programs in state `waiting` are reloaded into the scheduler with their triggers.
6. Start tick loop, event bus, IPC listener. Emit `daemon.started`.

### Resume

`vcli resume <program_id>` is intentionally narrow in v0. It is accepted only for programs whose semantics are recoverable from `source_json`, current screen state, and `body_cursor` alone:

- no watches
- no `elapsed_ms_since_true`
- no input step whose postcondition was still unresolved at crash time

Other programs fail fast with `not_resumable` and should be restarted from scratch.

For resumable programs, `vcli resume <program_id>` transitions a program that failed with `daemon_restart` back into `running`, starting at `body_cursor`. `--from-start` ignores the cursor and re-runs the body from index 0. Emits `program.resumed { from_step }`.

Edge case: if `body_cursor == len(body)` at failure time, resume re-enters `running` with body complete and immediately transitions to `completed` on the next tick.

### Graceful shutdown

`SIGTERM` / `vcli daemon stop`:

1. Stop accepting new IPC connections.
2. Finish the current tick, halt the loop.
3. Keep `running` programs in `running` state in the DB (they'll be transitioned to `failed(daemon_restart)` on next startup per the restart semantics above).
4. Close SQLite, unlink socket, emit `daemon.stopped`, exit 0.

`SIGKILL` skips this path — state changes are checkpointed on every transition (synchronous SQLite write), so the DB is never more than one tick behind reality.

### Error surfaces

| Surface | Consumer | Shape |
|---|---|---|
| CLI exit | Shell scripts | Exit code + stderr |
| IPC response | Client | `{ code, message, path? }` |
| Program-level failure | DB row + event | `program.failed { reason, step? }` |

Daemon-internal errors (`capture_failed`, `internal`) are logged via `tracing` at ERROR, surfaced in `vcli health`, and any program they affect is marked `failed(internal)` so it doesn't hang.

### Structured logging

`tracing` + `tracing-subscriber`, JSON in prod (detected via `!stderr.is_terminal()`), pretty in dev. File rotates daily, 7-day retention. `RUST_LOG` respected.

Trace records (program-semantic) and tracing logs (daemon-health) are intentionally separate: different audiences, different storage, different queries.

## Testing strategy

Three layers, each matched to the right boundary.

### Layer 1 — Unit tests (per crate, <1s)

- **`vcli-core`** — serde round-trips, canonical hashing stability.
- **`vcli-dsl`** — validation positive / negative cases; snapshot-style input → expected outcome.
- **`vcli-perception`** — each evaluator against canned `Frame` PNGs.
- **`vcli-runtime`** — internal scheduler helpers (e.g., arbiter resolution rules).
- **`vcli-ipc`** — frame codec, version field backwards-compat guard.
- **`vcli-store`** — migrations, asset dedup, GC correctness.

### Layer 2 — Scenario tests (deterministic; runtime crate)

The runtime crate has a `Scenario` harness:

```rust
let mut scenario = Scenario::new()
    .with_frames(load_canned_sequence("fixtures/yt_ad_sequence/"))
    .with_clock(TestClock::new("2026-01-01T00:00:00Z"))
    .with_program(include_str!("fixtures/yt_ad_skipper.json"));

scenario.run_until_event("ad_skipped", Duration::secs(30))?;

assert_eq!(scenario.program_state(pid), ProgramState::Running);
assert_trace_matches!(scenario.trace(), "fixtures/yt_ad_skipper.trace.jsonl");  // golden
```

- **Property assertions by default** (counts, final states, absence of deferreds).
- **Golden trace diffs** kept only for fully-deterministic scenarios (mock capture + test clock + mock input). `cargo test -- --update-goldens` regenerates them when an intentional change lands.
- **Every v0 feature has a scenario.** One-shot / persistent / `until_predicate` / `timeout_ms` watches; `while_true` retry behavior; body sequencing; `wait_for` timeout; `assert` failure; input postcondition success vs `novelty_timeout`; action conflict between two programs; predicate cache dedup; `elapsed_ms_since_true`; daemon restart produces `failed(daemon_restart)`; resumable-program recovery at `body_cursor`; `--from-start` resume.

### Layer 3 — E2E on real hardware

One canonical integration test, gated behind `cargo test --features e2e`:

- Opens a Safari tab on real YouTube (user-navigated; not scripted), loads a video with a pre-roll ad.
- Submits the YT ad skipper program.
- Watches `events --follow` for `ad_skipped` emitted by `watch.on_fire.emit`.
- Asserts the ad was actually dismissed via pixel-diff on the video region.
- Cancels the still-running program.

Property assertions only (no golden). README documents that this can flake on YouTube's UI/AB-test changes and the template asset may need updating — that's the whole point: the system is tracking a real moving target.

### Frame fixtures

Under `assets/fixtures/`, three categories:

1. **Synthetic** — tiny PNGs (200×200 white + 40×40 red, etc.) for predicate unit tests.
2. **UI captures** — real screenshots of target UIs, PNG, cropped, small.
3. **Sequences** — numbered directories (`yt_ad_sequence/000.png`, `001.png`, …) fed into `Scenario`.

Target total fixture weight: under 10MB.

### Fuzz testing

`cargo fuzz` target on `vcli-dsl`'s parser + validator. `arbitrary`-derived `Program` inputs. Runs manually / on a scheduled GitHub Action. Not a blocker for day-to-day development.

### Not in v0 testing

- No macOS GUI CI. Everything except `--features e2e` runs in CI.
- No VLM tests (VLM doesn't exist in v0).
- No automated perf regressions. Manual `vcli health` benchmark documented in README.

## Roadmap

### v0.1 — this spec

macOS, YT ad skipper works end-to-end. Ship a 60-second demo recording.

### v0.2 — OCR

Tier-3 evaluator worker, `ocr_text` and `ocr_number` predicate kinds. Demo: "wait for `Finished` in the iTerm build output, then run the next command."

### v0.3 — VLM

Tier-3 evaluator running a small local model (phi-3-vision / Qwen2.5-VL-3B / TBD) via `candle` or `ort`. `vlm` predicate kind with aggressive throttling (5s+ default between evals). Demo: "alert me if a Cloudflare human-verification appears."

### v0.4 — Windows backend

`windows-capture` for capture, `enigo` for input, `\\.\pipe\vcli` for IPC. Windows region kinds (class name, title regex). Document DirectInput limitation (Interception driver deferred). Demo: same YT ad skipper working on Windows with zero DSL changes.

### v0.5 — Planner (separate repo)

Thin LLM wrapper consuming the vcli event stream. Translates natural language goals into programs, submits them, reacts to completion/failure events. Uses public IPC.

### v0.6+ — pick-your-favorite

Subprogram composition • DirectInput games (Interception) • multi-display • remote daemon with auth • replay mode (`vcli replay <trace.jsonl>`) • read-only web dashboard • `.vcli` share bundles.

## Open questions / deferred decisions

- **Template matching backend.** `imageproc` NCC is the default; `opencv` crate is a fallback if perf or robustness demands.
- **Input backend on macOS.** `core-graphics` events vs `enigo` — decide at implementation time based on which gives cleaner modifier-key + international-keyboard behavior.
- **VLM model choice.** Decided at v0.3 entry.

## Appendix — decision log

| Decision | Choice | Section |
|---|---|---|
| Language | Rust (workspace) | Architecture |
| Platform v0 | macOS; trait abstractions for later Windows | Architecture |
| Canonical demo | YouTube ad skipper | v0 scope |
| DSL format | JSON | DSL |
| DSL shape | Named predicates + watches + body + triggers | DSL |
| Expression language | Dotted-path references only | DSL |
| Variables / conditionals in v0 | None | DSL |
| Perception tiers v0 | Tier 1 (`color_at`, `pixel_diff`, logical, `elapsed`) + Tier 2 (`template`) | Perception |
| Scheduler model | Sync 10fps tick, rayon for parallel eval, tokio only for IPC | Runtime |
| Arbitration on conflict | Drop losers (no re-queue); next tick re-decides | Runtime |
| Action semantics | Synchronous dispatch; body advances only after confirmation | Runtime |
| IPC | Unix socket, framed JSON, no auth | IPC |
| Persistence | SQLite + content-addressed assets + in-memory trace ring | Persistence |
| Restart policy | `running` → `failed(daemon_restart)`; opt-in `vcli resume` with `body_cursor` | Persistence |
| Testing | Property assertions + determinism-gated golden traces + DSL fuzz + real-Safari E2E | Testing |

## Review decisions — 2026-04-16 (plan-eng-review)

These 27 decisions from the 2026-04-16 engineering review update the spec above. When sections conflict, this appendix wins. Inline edits to the prose will happen during implementation; this appendix is the authoritative delta.

### Distribution (added to v0 scope)

- **0A.** Full v0 distribution: `launchd` plist + `vcli daemon install` command; GitHub Actions release workflow building **signed + notarized** macOS arm64/x86_64 binaries; Homebrew tap formula at `blaksmatic/homebrew-tap`; README quickstart.

### Architecture

- **1.1.** `vcli-core::canonical_json` module defines canonical form: object keys sorted lexicographically, numbers via `ryu`, UTF-8 NFC, no whitespace. `PredicateHash` and `source_json` both use canonical form. Round-trip tests mandatory.
- **1.2.** Daemon opens its IPC socket **only after** SQLite is migrated AND tick loop is running AND `daemon.started` has emitted. `vcli daemon start` blocks until the socket exists (10s timeout + clear error). An un-ready daemon does not answer `vcli health` — the socket simply doesn't exist yet.
- **1.3.** Predicate dependencies (region `relative_to` + logical composition) merge into one `PredicateGraph` at submit time. Single topological sort. Single cycle check. Unified evaluation walks leaves-first, memoizing results into `PerceptionCache`.
- **1.4.** macOS capture uses `screencapturekit-rs` (modern Apple API) — drop the legacy `core-graphics` `CGWindowListCreateImage` path. Spec line in §Perception pipeline → Window region resolution is corrected accordingly.
- **1.5.** Programs gain an optional `priority: integer` field (default 0). Arbiter tiebreak: priority desc → submission-time asc → program_id. No per-step priority.
- **1.6.** `Clock` trait moves from `vcli-runtime` to `vcli-core`. Every time-reading site (scheduler, perception cache throttle, `elapsed_ms_since_true`) takes `&dyn Clock`. `TestClock` deterministically drives the entire system.
- **1.7.** Event bus = per-client `tokio::sync::broadcast` channel, capacity 1024. Overflow → drop oldest + emit `stream.dropped { count, since }` so clients know they missed events. Terminal events (`program.completed`, `program.failed`) **also** persist in SQLite `events` table; clients can reconcile by re-querying on reconnect.
- **1.8.** Window geometry cache invalidates on macOS AX `kAXWindowMovedNotification` + `kAXWindowResizedNotification` per tracked window. 500ms TTL stays as belt-and-suspenders fallback.
- **1.9.** Memory bounds made concrete and enforced: template asset LRU = 128 decoded images (~64MB worst case), trace ring = 100k global + 10k per program. Current + peak exposed via `vcli health`.
- **1.10.** Five new ASCII diagrams (below) added to the spec.

### Code quality

- **2.1.** Every library crate defines its `Error` enum via `thiserror`. Both binaries (`vcli-daemon`, `vcli-cli`) use `anyhow::Result` at the top level. IPC layer maps library errors to stable codes.
- **2.2.** DSL error shape: `{code, message, path, line, column, span_len}`. Unknown names (predicate, region `kind`, action `kind`) get a Levenshtein-1 did-you-mean. CLI pretty-printer highlights the offending span with carets.
- **2.3.** `subprogram` kind removed entirely from the DSL. Not reserved, not schema-valid. Will be added in v0.6 via version bump (tagged enum variants are backward-compatible additions).
- **2.4.** On `vcli resume` at `body_cursor = N`, runtime re-evaluates step N-1's postcondition/assert/wait_for. If no longer truthy, fail with `resume_precondition_failed` so the caller can decide (restart from start or give up). Guarantees no silent step-skip on stale screen.
- **2.5.** Socket path resolution: macOS → `$TMPDIR/vcli-$UID.sock`. Linux → `$XDG_RUNTIME_DIR/vcli.sock` → `/run/user/$UID/vcli.sock` → `/tmp/vcli-$UID.sock` fallback chain.

### Testing

- **3.1.** Full CI in `.github/workflows/ci.yml`: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --all` on ubuntu-latest + macos-latest (excluding `--features e2e`), `cargo fuzz run dsl_validator -- -runs=10000` with 30s budget on PRs. Paired with the release workflow from 0A.

### Performance

- **4.1.** Validator **rejects** full-display `template` predicates unless the program opts in with `slow_budget: true`. Scheduler tracks per-tick cost histogram (`capture_ms`, `tier1_ms`, `tier2_ms`, `total_ms`), surfaced in `vcli health`. `daemon.pressure { tick_budget }` emits on 10 consecutive ticks over 90ms.
- **4.2.** Pyramid template matching lands in v0. At asset ingestion, precompute 2-level pyramid (full + ½ resolution). Matching runs ½-res first; if best match > (confidence − 0.05), refine at full-res around candidate.
- **4.3.** Capture runs at physical resolution; frames are immediately downsampled to logical (1x) resolution before perception. Templates are authored and stored at logical resolution. One resolution to reason about.
- **4.4.** SQLite PRAGMA on open: `journal_mode = WAL`, `synchronous = NORMAL`, `busy_timeout = 5000`, `cache_size = -32000` (32MB), `foreign_keys = ON`, `temp_store = MEMORY`.

### From the codex outside voice (cross-model consensus)

- **A.** `PerceptionCache` changes to `DashMap<PredicateHash, CacheEntry>`. `PredicateEvaluator::evaluate` takes `&self` (not `&mut cache`). Lock-free reads, sharded writes, safe under `rayon::par_iter`.
- **B.** Daemon architecture retained (not single-process v0). Decisions 1.1–4.4 + A–G de-risk the surface.
- **C.** `not_resumable` extended: any `sleep_ms` step, any throttled watch that has fired, any `elapsed_ms_since_true` with a recorded true transition → rejected at resume. Resume is narrow and safe.
- **D.** `on_schedule` trigger removed from v0 entirely. v0 triggers: `on_submit`, `on_predicate`, `manual`. Added in v0.6 alongside `WallClock` trait (see TODOS.md).
- **E.** Tick loop skips `capture.grab()` when no `running` program AND no `waiting on_predicate` program with active predicates. Idle daemon = near-zero CPU + no wasted permission prompts.
- **F1.** Explicit coordinate model: capture produces physical pixels → normalize to logical (1x) → AX returns logical points → input dispatch at logical → OS driver translates to physical. One conversion at the capture boundary, nowhere else.
- **F2.** Window disambiguation on 2+ matches: `window { app, title_contains }` resolves to the **oldest** window (lowest AX window ID). Optional `window_index: N` (0-based) selects the Nth match.
- **F3.** Postcondition-wait exclusivity: during the `within_ms` window after dispatching an input with a postcondition, the arbiter continues to allow OTHER programs' actions but blocks the same program's next action until the postcondition resolves (truthy or timeout).
- **F4.** `vcli-dsl` stays pure JSON → validated AST. Asset path resolution + hashing move out of DSL into a new thin `vcli-daemon::submit` module that orchestrates `dsl::validate` → `store::ingest_assets` → `store::rewrite_program`. DSL crate has no filesystem or hash dependencies.
- **G.** Demo strategy: YT ad skipper stays as the headline, recognizable 60-second demo. Add a second E2E against a stable local desktop app (Calculator, Preview, or a small in-repo test harness) as the regression target that isn't subject to YouTube's UI drift.

## Diagrams

### Tick loop dataflow

```
┌──────────────────────────────────────────────────────────────────┐
│                        TICK BOUNDARY (100ms)                      │
└──────────────────────────────────────────────────────────────────┘
         │
         ▼
 ┌───────────────────┐    empty?      ┌──────────────────────┐
 │ Any program need  │───── YES ─────▶│ SKIP capture + eval  │──▶ sleep
 │ perception now?   │                │ (Decision E)          │
 └────────┬──────────┘                └──────────────────────┘
          │ NO
          ▼
 ┌───────────────────────┐   >90ms 10x    ┌─────────────────────┐
 │ capture.grab()        │ ─────────────▶ │ daemon.pressure     │
 │ (ScreenCaptureKit)    │                │ { tick_budget } (4.1)│
 │ physical → logical    │                └─────────────────────┘
 │ (Decision 4.3, F1)    │
 └────────┬──────────────┘
          │  Arc<Frame>
          ▼
 ┌──────────────────────────────────────────────┐
 │ PredicateGraph (unified DAG, Decision 1.3)    │
 │  ┌──────────────┐  ┌──────────────┐           │
 │  │  Tier-1 eval │  │ Tier-2 eval  │           │
 │  │ (inline)     │  │ rayon∥,      │           │
 │  │ color_at,    │  │ respects     │           │
 │  │ pixel_diff,  │  │ throttle_ms, │           │
 │  │ logical,     │  │ template     │           │
 │  │ elapsed      │  │ (pyramid 4.2)│           │
 │  └──────┬───────┘  └──────┬───────┘           │
 │         │                 │                    │
 │         └────────┬────────┘                    │
 │                  ▼                             │
 │      DashMap<PredicateHash, CacheEntry>        │
 │      (Decision A — interior mutability,        │
 │       safe under par_iter)                     │
 └──────────┬──────────────────────────────────────┘
            │ PredicateResult per predicate
            ▼
 ┌────────────────────────────────────────────────┐
 │ For each waiting program:                       │
 │   trigger.maybe_fire(results, clock)            │
 │                                                  │
 │ For each running program:                       │
 │   program.advance(results, clock)                │
 │   → pending actions (body + fired watches)      │
 └────────┬────────────────────────────────────────┘
          │ Vec<PendingAction>
          ▼
 ┌────────────────────────────────────────────────┐
 │ ActionArbiter (Decision 1.5, F3)               │
 │   sort by priority desc → submit_time → pid    │
 │   pick at most: 1 mouse, 1 keyboard            │
 │   losers: emit action.deferred, no re-queue    │
 │   blocked-on-own-postcondition: skipped        │
 └────────┬────────────────────────────────────────┘
          │ chosen actions (≤2)
          ▼
 ┌────────────────────────────────────────────────┐
 │ input.dispatch(action) — sync confirm            │
 │   postcondition? mark program waiting for it.   │
 │   no postcondition? step complete.              │
 └────────┬────────────────────────────────────────┘
          │
          ▼
 ┌────────────────────────────────────────────────┐
 │ event_bus.drain_and_publish()                   │
 │   → broadcast(1024) per client (Decision 1.7)   │
 │   → overflow: drop oldest + stream.dropped      │
 │   → terminal events → SQLite events table      │
 │                                                  │
 │ trace.append(tick_record) — bounded ring (1.9)  │
 │                                                  │
 │ SQLite writes (WAL + NORMAL, Decision 4.4):    │
 │   → state transitions                           │
 │   → body_cursor bumps                           │
 └────────┬────────────────────────────────────────┘
          │
          ▼
      sleep_until(next tick boundary)
```

### Merged predicate DAG (Decision 1.3)

```
    Sources of dependency edges:
      (R) region relative_to → predicate
      (L) logical of → predicate
      (E) elapsed_ms_since_true → child predicate

                  ┌──────────────────┐
                  │   skip_visible   │──(L)──┐
                  │   (template)     │       │
                  └────────┬─────────┘       │
                           │(R region anchor)│
                           ▼                  ▼
                  ┌──────────────────┐   ┌──────────┐
                  │ item_row_visible │   │ skip_gone│
                  │ (template,       │   │ (not)     │
                  │  relative_to)    │   └──────────┘
                  └──────────────────┘
                           │(L)
                           ▼
                  ┌──────────────────┐
                  │   ready_to_buy   │
                  │    (all_of)      │
                  └────────┬─────────┘
                           │(E)
                           ▼
                  ┌──────────────────┐
                  │ stable_for_500ms │
                  │ (elapsed_ms...)  │
                  └──────────────────┘

    At submit:
      1. Build DAG from all edge sources.
      2. Topological sort. Reject cycles with invalid_program.
      3. Store alongside source_json.

    At tick:
      Evaluate in topological order (leaves first).
      Tier-1 inline, Tier-2 on rayon pool.
      DashMap cache dedups across programs by PredicateHash
      (canonical JSON, Decision 1.1).
      Tier-3 (v0.2+): last-cached result only; no inline eval.
```

### Program lifecycle (complete)

```
            vcli submit program.json
                     │
                     ▼
              ┌───────────┐
              │  pending  │
              └─────┬─────┘
                    │ daemon ready (Decision 1.2)
                    ▼
              ┌───────────┐ ◀──────────────────────────┐
              │  waiting  │                             │
              └─────┬─────┘                             │
                    │ trigger fires (on_submit / on_predicate / manual)
                    ▼                                   │
              ┌───────────┐                             │
        ┌────▶│  running  │────────────┐                │
        │     └─────┬─────┘            │                │
        │           │                  │                │
        │    body   │ complete         │ step error     │
        │    OR     │ (non-empty body) │ / assert fail  │
        │    last   ▼                  │ / wait_for     │
        │   watch ┌───────────┐        │   on_timeout=fail
        │   removed│ completed │        │ / postcondition
        │           └───────────┘        │   on_timeout=novelty
        │                                │   (→ program.novelty
        │                                │      then failed(novelty_timeout))
        │    unblock                     │ / program-level timeout_ms
        │  (future)                      │ / daemon_restart
        │    ┌────────┐                  ▼
        └────│ blocked│           ┌───────────┐
             └────────┘           │  failed   │
             (reserved, no        └───────────┘
              v0 transitions)           ▲
                                        │
              cancel request            │
                 │                       │
                 ▼                       │
              ┌───────────┐              │
              │ cancelled │              │
              └───────────┘              │
                                         │
      vcli resume <id>                   │
      (only if resumable &                │
       failed(daemon_restart))            │
                │                         │
                ▼                         │
          re-eval step N-1                │
          postcondition (Decision 2.4)    │
                │                         │
       ┌────────┴─────────┐               │
       │                  │               │
       ▼ truthy           ▼ not truthy    │
    running           failed(resume_      │
                      precondition_failed)┘
```

### IPC request/event flow

```
┌───────────┐    ┌───────────────────────────────────────────────┐
│  vcli CLI │    │                  vcli daemon                   │
└─────┬─────┘    └────────────────────────────────────────────────┘
      │
      │  1. connect  $TMPDIR/vcli-$UID.sock (Decision 2.5)
      │                                          (fails if not ready — 1.2)
      ├─────────────────────────────────────────▶│
      │                                          │
      │  2. send framed { id, op, params }       │
      │     u32 BE length + UTF-8 JSON           │
      ├─────────────────────────────────────────▶│
      │                                          │
      │                              ┌───────────┤
      │                              │ op dispatcher
      │                              │   submit / list / status /
      │                              │   cancel / start / resume /
      │                              │   logs / events / trace /
      │                              │   health / gc / shutdown
      │                              └───────────┤
      │                                          │
      │                                          │ for submit:
      │                                          │   dsl::validate (pure)
      │                                          │ → store::ingest_assets
      │                                          │ → store::rewrite_program
      │                                          │ → runtime::enqueue
      │                                          │   (Decision F4)
      │                                          │
      │  3a. response { id, ok, result | error } │
      │◀─────────────────────────────────────────┤
      │                                          │
      │  3b. (streaming ops: logs, events)       │
      │     per-client broadcast(1024)           │
      │      { stream, type, program_id?, data } │
      │◀─ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ┤
      │                                          │
      │  3c. overflow: stream.dropped            │
      │     { count, since } (Decision 1.7)      │
      │◀─ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ═ ┤
      │                                          │
```

### Restart + resume sequence

```
Phase A — daemon startup
─────────────────────────────────────────────────
  1. Read config (config.toml if present)
  2. Open SQLite, run migrations (vcli-store)
  3. Scan assets/ vs program_assets refs → log orphan count
  4. Transition state:
       running  → failed(daemon_restart)
       waiting  → reload into scheduler with trigger
  5. Start tick loop
  6. Start event bus
  7. Open IPC socket ◀── first observable "ready" (Decision 1.2)
  8. Emit daemon.started
  9. (optional) GC if last run >7 days ago

Phase B — optional user-initiated resume
─────────────────────────────────────────────────

  vcli resume <program_id> [--from-start]
        │
        ▼
  Is program in failed(daemon_restart)?
        │
     YES│    NO
        │    └──▶ error: bad_state_transition
        ▼
  Check resumability (Decision 2.4 + C):
    - no watches
    - no elapsed_ms_since_true WITH recorded transition
    - no sleep_ms step
    - no throttled watch that has fired
    - no postcondition unresolved at crash time
        │
     YES│    NO
        │    └──▶ error: not_resumable
        ▼
  --from-start?
        │
      NO│    YES
        │    └──▶ body_cursor := 0, transition → running
        ▼
  Re-evaluate step (body_cursor - 1)'s postcondition/assert/wait_for
        │
   truthy│   not truthy
        │    └──▶ error: resume_precondition_failed
        ▼
  Transition → running
  Emit program.resumed { from_step }
```

## Backend wiring decisions — 2026-04-22 (post-ship)

Plan-4 (`vcli-daemon`) shipped (PR #10) with `MockCapture::empty()` and `MockInputSink::new()` hardcoded in `default_runtime_factory`. The released binary cannot capture screens or synthesize input; every tick logs `MockCapture has no screen frames configured`. This appendix specifies the wiring of the real macOS backends so the binary delivers the v0 thesis from §Summary. Implemented by `docs/superpowers/plans/2026-04-22-vcli-daemon-real-backends.md`.

### Decisions

- **B1.** On `target_os = "macos"`, `default_runtime_factory` constructs `vcli_capture::macos::MacCapture::new()` (fallible — TCC) and `vcli_input::macos::CGEventInputSink::new(kill)` (infallible). On every other platform, the factory keeps the v0 mocks. Windows backends arrive in v0.4 per the README status table (`vcli-capture` / `vcli-input` Windows stubs are present today; real implementations are deferred); this appendix does not bring them forward.
- **B2.** The kill-switch tap (`Cmd+Shift+Esc` halts input dispatch — Decision B / spec §Safety) starts via `vcli_input::macos::spawn_kill_switch_listener(kill)` and its handle lives as long as `RuntimeBackends`. Dropping `RuntimeBackends` at daemon shutdown tears down the tap thread cleanly. Non-macOS builds carry no listener.
- **B3.** `RuntimeBackends` gains a type-erased field `_shutdown_guard: Option<Box<dyn std::any::Any + Send + Sync>>`. The macOS factory parks the kill-switch handle here; mock factories leave it `None`. Type erasure avoids a cfg-gated public type in the daemon crate's API surface — callers (including tests) never need to name `KillSwitchListenerHandle`. The `Any` box's `Drop` runs the inner handle's `Drop`, which is sufficient.
- **B4.** Daemon startup logs `vcli_input::permissions::probe()` at INFO level on every boot (one structured `tracing::info!` event with the report fields). A user who hits a `permission_denied` runtime error can scroll back one log line to see whether Accessibility / Input Monitoring were granted at boot. The probe is non-prompting — it never opens a system dialog.
- **B5.** `MacCapture::new()` returning `CaptureError::PermissionDenied` is mapped to a new `DaemonError::BackendInit { backend: &'static str, reason: String }` variant. The daemon refuses to start (exit code 1, clear stderr message naming Screen Recording and pointing at `System Settings → Privacy & Security → Screen Recording`) rather than booting into a permanently-failing tick loop. Today's behavior — a daemon that endlessly logs `MockCapture has no screen frames configured` — is the worst of both worlds.
- **B5a (post-merge fix, 2026-04-22).** `factory_macos::build()` calls `vcli_capture::permission::request_screen_recording_permission()` (FFI: `CGRequestScreenCaptureAccess`) before constructing `MacCapture`. This triggers the macOS Screen Recording system prompt the first time an unprivileged binary attempts capture. Without this, `MacCapture::new()`'s probe (`CGPreflightScreenCaptureAccess`) returns `Denied` silently, the daemon fails clean per B5, and the user never sees a system dialog — they have to manually add the binary in System Settings, which most users won't know to do. The request call's result is ignored: it's async from the user's perspective, so `MacCapture::new()` immediately after will still observe `Denied` on the first run; the user grants in the dialog and the *next* daemon start succeeds. Input Monitoring's prompt is triggered automatically by the first `CGEventTap::new()` call inside `spawn_kill_switch_listener`, so no equivalent fix is needed there.
- **B6.** This work does **not** address the scheduler-vs-store state divergence bug found alongside it: in-memory `RunningProgram.state` is never persisted, so `vcli list` always reports `pending` for everything. That fix needs its own plan because it touches the runtime crate's commit boundary, not the daemon binary. Filed in TODOS.md.

### Out of scope for plan-6

- Wiring real backends on non-macOS (covered by v0.4 per Decision G).
- A `--mock-backends` CLI flag for dev / CI overrides (file as TODOS.md follow-up if needed; for now `RuntimeFactory` injection in the test crate covers all in-tree mock needs).
- The `vcli list` state-persistence fix (separate plan; see B6).
- Surfacing the permission report in `vcli health` JSON output (the log line in B4 is the v0 minimum; a structured field is a follow-up).

### How this fits the existing decisions

- Extends Decision **1.4** (SCK backend) by actually instantiating it in the binary that ships.
- Honors Decision **B** (kill switch) — the `MockInputSink` in v0 ignored the kill switch entirely because nothing was listening; B2 fixes that.
- Keeps the `RuntimeFactory` injection pattern from the v0 daemon plan intact — daemon unit tests still construct mocks via the factory parameter, with no behavior change.

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 0 | — | — |
| Codex Review | `/codex review` | Independent 2nd opinion | 1 | issues_found | 10 findings, all integrated |
| Eng Review | `/plan-eng-review` | Architecture & tests (required) | 1 | CLEAR (PLAN) | 27 issues, 0 critical gaps |
| Design Review | `/plan-design-review` | UI/UX gaps | 1 | not_applicable | no UI scope |
| DX Review | `/plan-devex-review` | Developer experience gaps | 0 | — | — |

- **CODEX:** 10 findings raised, 7 new vs the review, 3 overlap. All resolved by Decisions A–G in the "Review decisions — 2026-04-16" appendix.
- **CROSS-MODEL:** Eng review and codex converged on 3 issues (DPI, resume safety, window geometry). Codex caught 7 additional issues the review missed (most critical: `&mut PerceptionCache` + rayon conflict, `on_schedule` clock mismatch, DSL crate IO layering). All folded into the decision ledger.
- **UNRESOLVED:** 0
- **VERDICT:** ENG CLEARED — ready to implement.
