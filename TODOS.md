# TODOS

## v0.1 post-ship — Benchmark harness

**What:** `cargo bench` suite measuring per-tick cost at p50/p95/p99 under N-program load.
**Why:** Decision 4.1 added a per-tick cost histogram in `vcli health`, but no regression guard exists. Benchmarks catch perf regressions before they ship.
**Pros:** Regression detection; concrete numbers for README claims.
**Cons:** ~3h to set up `criterion`; bench runs need stable hardware.
**Context:** Scheduler tick loop has a documented 100ms budget (10fps). Breaking the budget silently degrades the reactive promise. Needs a criterion bench that spins N mock-frame sequences and measures tick cost distribution. Should run on a pinned GitHub Actions macOS runner or be run manually.
**Depends on / blocked by:** v0.1 shipped.

## v0.2 — Full resume checkpointing

**What:** Persist watch fire counts, throttle last-fire times, and elapsed-since-true transition times to SQLite on every change. Enables resume of programs currently rejected as `not_resumable`.
**Why:** Decision C (from /plan-eng-review 2026-04-16) tightened resume to "safe and narrow" — any `sleep_ms`, any fired-throttled watch, or any elapsed transition currently makes a program `not_resumable`. Fuller recovery is a v0.2 lift.
**Pros:** Long-running reactive programs (YT ad skipper, notification watchers) survive daemon restart with full semantics preserved.
**Cons:** 5-10x SQLite write amplification per tick for busy programs. Needs careful PRAGMA tuning and possibly batched commits. ~6h work.
**Context:** Current resume contract = re-eval step N-1 postcondition only. That's enough for simple body-only programs but drops reactive state. Full checkpointing needs a new `runtime_state` table keyed by (program_id, state_kind, key) and a `CheckpointWriter` that batches writes between ticks.
**Depends on / blocked by:** v0 shipped. Should come after v0.2's OCR work so we don't double-migrate the schema.

## v0.2 — WallClock trait + `on_schedule` trigger

**What:** Add a `WallClock` trait for wall-clock + timezone reads; add `on_schedule` as a trigger kind with cron-ish syntax + IANA timezone.
**Why:** Decision D (from /plan-eng-review 2026-04-16) removed `on_schedule` from v0 triggers because it was architecturally incompatible with the monotonic `Clock` trait. A dual-clock model solves it cleanly.
**Pros:** "Skip YouTube ads every day at 9pm" and other scheduled-reactive programs become expressible.
**Cons:** ~200 LoC + `chrono-tz` dep. Cron parsing has edge cases (DST transitions, leap seconds). Needs IANA tz ID resolution.
**Context:** v0 ships with only `on_submit`, `on_predicate`, `manual`. The monotonic `Clock` lives in `vcli-core`. `WallClock` should live beside it as a sibling trait, with a `SystemWallClock` prod impl and a `TestWallClock` for determinism in scheduling tests.
**Depends on / blocked by:** v0 shipped.
