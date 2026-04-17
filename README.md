# vcli — Vision CLI

A local, persistent screen-control runtime that AI agents command through declarative JSON programs.

The thesis: an agent shouldn't need one LLM call per frame. Submit a JSON *program* once, and a long-running local daemon keeps watching the screen at 10 fps, reacting in real time — click the "Skip Ad" button whenever YouTube shows one, refill the cart when it empties, retry a failing upload. The agent is the slow planner; vcli is the fast reflex.

## Status

**v0.1 (pre-alpha).** The seven library crates that make up the runtime are on `master`; the daemon binary and user-facing CLI are still to come.

| Crate | Role | Status |
| --- | --- | --- |
| `vcli-core` | Shared types, canonical JSON, hashing, event/error taxonomy | ✅ |
| `vcli-dsl` | Parse + validate JSON programs into `vcli-core::Program` | ✅ |
| `vcli-store` | SQLite persistence + content-addressed asset store + GC | ✅ |
| `vcli-ipc` | tokio Unix-socket + length-prefixed JSON, server/client scaffolds | ✅ |
| `vcli-input` | `InputSink` trait + macOS CGEvent backend + kill-switch chord | ✅ |
| `vcli-capture` | `Capture` trait + macOS ScreenCaptureKit + `MockCapture` | ✅ |
| `vcli-perception` | Tier-1/2 predicate evaluators + per-tick DashMap cache | ✅ |
| `vcli-daemon` | Tick loop wiring capture → perception → scheduler → input | ⏳ |
| `vcli-cli` | `vcli submit`, `list`, `cancel`, `logs` | ⏳ |

macOS is the primary target; Windows ships in v0.4 (each platform-specific crate exposes a Windows stub so the workspace builds cross-platform today).

## Canonical demo

[`fixtures/yt_ad_skipper.json`](fixtures/yt_ad_skipper.json) is a minimal program that watches Safari/YouTube for the "Skip Ad" button and clicks it. Once the daemon lands, it will be the end-to-end smoke test.

## Build & test

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The workspace pins Rust via [`rust-toolchain.toml`](rust-toolchain.toml) and uses `#![forbid(unsafe_code)]` everywhere except for the thin FFI layers in `vcli-input` (CGEvent) and `vcli-capture` (ScreenCaptureKit).

## Docs

- [`docs/superpowers/specs/2026-04-16-vcli-design.md`](docs/superpowers/specs/2026-04-16-vcli-design.md) — authoritative v0 design spec.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — system overview, data flow, tick loop, key design decisions.
- [`AGENT.md`](AGENT.md) — instructions for AI agents working in this repo (TDD, commit cadence, gates).
- [`docs/superpowers/plans/`](docs/superpowers/plans/) — per-crate implementation plans.

## License

MIT — see [`LICENSE`](LICENSE).
