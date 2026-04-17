# AGENT.md — Instructions for AI agents working on vcli

This file is the top-priority reference for any AI agent (Claude Code, Codex, Cursor, etc.) opening this repo. Read it before you touch code.

For the *what* and *why* of the system, see [`README.md`](README.md) and [`ARCHITECTURE.md`](ARCHITECTURE.md). For the authoritative design, see [`docs/superpowers/specs/2026-04-16-vcli-design.md`](docs/superpowers/specs/2026-04-16-vcli-design.md).

## How we build things here

vcli is built **spec → plan → implement**, never freestyle:

1. A **design spec** in `docs/superpowers/specs/` captures decisions with rationale.
2. A **plan** in `docs/superpowers/plans/` breaks the spec into TDD tasks — each 2–5 minutes, each with exact code, each ending in a commit.
3. An **implementer** executes the plan task-by-task. Failing test first, minimal impl, verify, commit.

If you are starting new work that doesn't fit an existing plan, write a spec section and a plan first. Do not skip straight to code.

## Commit discipline

- **One commit per plan task.** Never squash, never batch.
- **Commit message format:** `<crate>: short summary in imperative mood` — e.g., `vcli-dsl: cycle detection over merged predicate graph`.
- Always include `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` (or equivalent for your model) when working autonomously.
- **Never use `--no-verify`, `--force`, or `--force-with-lease`** unless the human explicitly asks. If a hook or check fails, fix the underlying issue.
- `git add` specific files by name. **Never** `git add -A` or `git add .`.

## Gates that must stay green

All three of these must pass in every commit and on every push:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

CI enforces the same three on both `ubuntu-latest` and `macos-latest`. A warning is an error. There are no `#[allow(...)]` escape hatches without a `# Why:` comment above them.

Integration tests that require macOS TCC permissions (Screen Recording, Accessibility, Input Monitoring) must be marked `#[ignore]` so unattended runs stay green. Humans unignore them for manual verification.

## What you may not change

- **`crates/vcli-core/`** — the foundational types. Every other crate depends on it. Changes require a dedicated plan (not drive-bys from another lane).
- **The design spec** — if you believe the spec is wrong, open a decision log entry under "Review decisions" in the spec rather than changing code that violates it.
- **`#![forbid(unsafe_code)]`** at crate roots — if you need FFI, scope the `unsafe` inside a well-named helper function and document invariants.

## Code conventions

- Rust edition 2021, MSRV 1.75 (pinned via `rust-toolchain.toml`).
- `#![forbid(unsafe_code)]` everywhere except the thin FFI wrappers in `vcli-input` (CGEvent) and `vcli-capture` (ScreenCaptureKit).
- `#![warn(missing_docs)]` and `#![warn(clippy::pedantic)]` on library crates. `# Panics` and `# Errors` doc sections where appropriate.
- Errors use `thiserror`, carry typed variants, and map to a stable `code()` string that matches the spec's error taxonomy.
- Serde: tagged enums with `snake_case` for DSL kinds (`#[serde(tag = "kind", rename_all = "snake_case")]`), `#[serde(untagged)]` only where the spec calls for it (`Target`, `WatchWhen`).
- Tests always use `tempfile::tempdir()` for filesystem state. **Never** create files in the repo or cwd.
- **Never mutate process-global state** (`std::env::set_var`, `std::env::remove_var`) in tests without recognizing it races with every other parallel test. If you must, use a real directory that exists (`/tmp`) so callers that read the mutated var during the race window don't blow up. See PR #5's `mac_uses_tmpdir_branch` fix for precedent.

## Running things locally

```bash
# Full workspace gate (what CI runs):
cargo test --workspace --locked && \
cargo clippy --workspace --all-targets -- -D warnings && \
cargo fmt --all -- --check

# Just one crate:
cargo test -p vcli-dsl
cargo clippy -p vcli-dsl --all-targets -- -D warnings

# Run TCC-gated tests (macOS, requires Screen Recording / Accessibility grants):
cargo test -p vcli-capture -- --ignored
cargo test -p vcli-input -- --ignored

# Example: capture one frame (manual TCC verification):
cargo run -p vcli-capture --example capture_once -- --save /tmp/out.png
```

## Working in parallel (advanced)

Multiple lanes can proceed at once via `git worktree add ../vcli-lane-X -b implement/plan-N-vcli-<name> <base-branch>`. Each worktree is its own checkout; changes there don't affect the main one. Coordination cost is the workspace `Cargo.toml` members list — expect a small merge conflict each time a new crate lands on master, and resolve by listing all members.

## When in doubt

- Read the task in the plan exactly once, top to bottom, before writing anything.
- If the plan shows code, write that code. If the plan and the current reality disagree on a type name or signature, trust reality (`cargo check` will tell you), correct the plan inline via a one-line commit body addendum, and keep going.
- If you hit a hard blocker (missing crate on crates.io, spec-vs-plan contradiction you can't resolve, a test that can't fail because the "failing test" actually compiles green) — **stop**. Do not hack around it. Report what you're stuck on and which task you're on.

Taste: three similar lines is better than a premature abstraction. No error handling for scenarios that can't happen. No comments that describe *what* the code does (the identifier already does). No documentation for hypothetical future requirements. Delete unused code instead of leaving `// removed` breadcrumbs.
