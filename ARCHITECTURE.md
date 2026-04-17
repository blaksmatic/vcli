# vcli Architecture

This document is the system-level overview. Authoritative details — decisions with rationale, wire formats, error taxonomy, roadmap — live in [`docs/superpowers/specs/2026-04-16-vcli-design.md`](docs/superpowers/specs/2026-04-16-vcli-design.md).

## Core thesis

Agents that drive a screen through one LLM call per frame are slow, expensive, and brittle. The same work splits cleanly into two layers:

- **Planner (remote LLM, slow).** Composes a *program* — a JSON document naming the predicates to watch and the actions to take when they fire — and submits it once.
- **Runtime (vcli, local, fast).** Evaluates the program against captured screen state at 10 fps, fires actions reactively, persists state and events, and answers queries about what it did. No model in the hot path.

One LLM call per task instead of per frame. Cheaper, faster, and orders of magnitude more reliable for the class of work that fits the reactive pattern ("skip ads", "buy the cheapest option", "dismiss the cookie banner whenever it appears").

## Workspace layout

```
vcli/
├── crates/
│   ├── vcli-core/         shared types, canonical JSON, predicate hashing, event/error taxonomy
│   ├── vcli-dsl/          parse + validate JSON programs into vcli-core::Program
│   ├── vcli-store/        SQLite persistence + content-addressed asset store + GC
│   ├── vcli-ipc/          tokio Unix-socket server + client; framed JSON; Handler trait
│   ├── vcli-input/        InputSink trait + macOS CGEvent backend + kill-switch CGEventTap
│   ├── vcli-capture/      Capture trait + macOS ScreenCaptureKit + MockCapture + Windows stub
│   ├── vcli-perception/   Tier 1/2 evaluators + DashMap per-tick cache + PerceptionState
│   ├── vcli-daemon/       (TBD) tick loop wiring the above
│   └── vcli-cli/          (TBD) thin client: vcli submit/list/cancel/logs
├── docs/superpowers/
│   ├── specs/             design spec (authoritative)
│   └── plans/             per-crate implementation plans
└── fixtures/              example programs (yt_ad_skipper.json)
```

Each crate is a clear boundary with its own error type, unit tests against mocks, and integration tests that exercise real behavior (macOS integration tests are `#[ignore]`-gated so unattended CI runs without TCC permissions stay green).

## Data flow

```
        ┌─────────────────┐
        │   LLM planner   │  (remote)
        └────────┬────────┘
                 │  JSON program
                 ▼
        ┌─────────────────┐
 Unix   │    vcli-cli     │  thin client
 socket └────────┬────────┘
                 │  Request::Submit { program }
                 ▼
        ┌─────────────────┐   ┌──────────────┐
        │   vcli-ipc      │──▶│  vcli-store  │  persist canonical program + events
        │  (Server)       │   └──────────────┘
        └────────┬────────┘
                 │ handler dispatch
                 ▼
        ┌──────────────────────────────────────┐
        │          vcli-daemon (TBD)           │
        │  ┌─────────────────────────────────┐ │
        │  │     10 fps tick loop            │ │
        │  │                                 │ │
        │  │  ┌──────────────┐  Frame        │ │
        │  │  │ vcli-capture │──────────┐    │ │
        │  │  └──────────────┘          ▼    │ │
        │  │                    ┌─────────────┐
        │  │                    │ vcli-       │
        │  │                    │ perception  │◀── DashMap cache (clear per tick)
        │  │                    └──────┬──────┘
        │  │                           │ PredicateResult
        │  │                           ▼
        │  │                    ┌─────────────┐
        │  │                    │  scheduler  │ arbitrate actions, confirm before drop
        │  │                    └──────┬──────┘
        │  │                           │ InputAction
        │  │  ┌────────────┐           ▼
        │  │  │ vcli-input │◀──────────┘
        │  │  └────────────┘  CGEvent
        │  └─────────────────────────────────┘
        └──────────────────────────────────────┘
                         │
                         ▼
                   Event stream → vcli-store (append) → CLI subscribers
```

The LLM never enters the inner loop. Capture → perception → decide → act runs 100 ms end-to-end on commodity hardware.

## The tick

Every 100 ms (10 fps), the scheduler runs exactly one pass:

1. `vcli-capture` grabs a frame (full screen or a specific window).
2. `vcli-perception.clear()` empties the per-tick `DashMap<PredicateHash, PredicateResult>` cache.
3. For each running program, each watch's `when` predicate is evaluated. Evaluation is memoized by canonical-predicate SHA-256 hash — if two programs share the same `template` predicate, the image only gets scanned once.
4. Firing watches produce `InputAction`s. The scheduler arbitrates (priority-based, no re-queue of losers), dispatches synchronously, and confirms the action landed before advancing.
5. State changes, watch fires, and action outcomes emit `Event`s, which `vcli-store` appends and `vcli-ipc` streams to subscribed CLI clients.

Cross-tick state (prior frames for `pixel_diff`, first-true timestamps for `elapsed_ms_since_true`) lives in `PerceptionState`, separate from the per-tick cache.

## Predicate tiers

Predicates are layered by cost. A watch's body short-circuits cheaper predicates before expensive ones so the hot path stays cheap.

| Tier | Predicates | Typical cost |
| --- | --- | --- |
| 1 | `color_at`, `pixel_diff`, logical (`all_of` / `any_of` / `not`), `elapsed_ms_since_true` | microseconds — pure pixel math |
| 2 | `template` (imageproc NCC) | single-digit milliseconds |
| 3 *(post-v0)* | OCR, VLM calls | hundreds of ms, network-bound |

Only Tiers 1 and 2 ship in v0. Tier 3 is explicitly deferred — reactive runtimes do not depend on model inference in the inner loop.

## Persistence & restart

- `vcli-store` uses SQLite with WAL + foreign keys. Programs, events, and traces live in the DB; large binary blobs (template images, captured frames attached to traces) are content-addressed and live on disk at `~/.local/share/vcli/assets/sha256/<aa>/<bb>/<hex>.<ext>`.
- On daemon start, any program left in `running` is transitioned to `failed(reason="daemon_restart")`, preserving `body_cursor` for an opt-in `vcli resume`. We do **not** automatically resume — narrow resumability is deliberate; restart after an OS reboot means the world has changed.
- GC runs on a 7-day retention window for completed programs and orphaned assets.

## IPC

Unix domain socket at `$TMPDIR/vcli-$UID.sock` (macOS) / `$XDG_RUNTIME_DIR/vcli.sock` (Linux). Every request is a length-prefixed JSON frame (u32 big-endian length, then UTF-8). No auth — single-user, local-only by design. Event and trace streams use the same framing with an `end_of_stream` sentinel.

## Safety

- `#![forbid(unsafe_code)]` at the root of every pure-Rust crate (core, dsl, store, ipc, perception). Only `vcli-input` and `vcli-capture` contain FFI `unsafe`, scoped to the thinnest possible wrapper around CGEvent / ScreenCaptureKit.
- **Kill switch:** `Cmd+Shift+Esc` globally halts input dispatch — implemented as a `CGEventTap` listener in listen-only mode, flipping an `Arc<AtomicBool>` that every `InputSink` method checks before posting events.
- No LLM calls mean no prompt-injection vector into the runtime itself. The submitted program is validated against a typed schema (`vcli-dsl`) before it ever touches the scheduler.

## What's *not* in v0

- OCR or VLM predicates (planned for v0.5+)
- `subprogram` composition (decided out; programs are flat in v0)
- `on_schedule` triggers (decided out; triggers are watch-based only)
- Auth or sandboxing (single-user local runtime)
- Automatic resume after daemon restart (opt-in via `vcli resume` only)
- Windows input/capture (trait abstractions are in place; macOS ships first, Windows lands in v0.4)

See the spec's "Review decisions — 2026-04-16" appendix for the full list with rationale.
