# vcli template-assets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make template predicates functional end-to-end by resolving submitted template image paths, ingesting them into the content-addressed store, rewriting stored programs to `sha256:<hash>` references, and passing loaded asset bytes to the runtime.

**Architecture:** Keep `vcli-dsl` pure as required by Decision F4. The CLI sends the program file's parent directory as optional submit metadata. The daemon performs all asset IO in a new `assets` helper module before inserting the program row, then sends the rewritten `Program` plus an asset byte map to `vcli-runtime`. The runtime stores assets on each `RunningProgram` and passes them into trigger/watch/body perception calls instead of the current empty map.

**Tech Stack:** Rust 2021, existing `serde_json`, `vcli-core`, `vcli-ipc`, `vcli-store`, `vcli-daemon`, `vcli-runtime`, and `vcli-perception`. No new dependencies.

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md` Decision F4 and the asset store sections.

---

## File structure

```
crates/vcli-ipc/src/wire/request.rs          # add Submit.base_dir metadata
crates/vcli-cli/src/commands/submit.rs       # send canonical parent dir
crates/vcli-daemon/src/assets.rs             # new daemon-only asset materialization helper
crates/vcli-daemon/src/handler.rs            # call helper during submit/resume
crates/vcli-daemon/src/lib.rs                # expose module
crates/vcli-runtime/src/command.rs           # add assets to SubmitValidated/ResumeRunning
crates/vcli-runtime/src/program.rs           # store per-program asset bytes
crates/vcli-runtime/src/scheduler.rs         # pass program assets into perception
```

## Task 1: preserve submit base directory over IPC

**Files:**
- Modify: `crates/vcli-ipc/src/wire/request.rs`
- Modify: `crates/vcli-cli/src/commands/submit.rs`
- Modify tests using `RequestOp::Submit`

- [ ] **Step 1: Write failing IPC and CLI tests**

Add an IPC roundtrip assertion in `request.rs` that `base_dir` serializes under `params` and defaults to `None`. Add a CLI e2e fake-daemon assertion that a submitted file under a tempdir sends that tempdir as `base_dir`.

- [ ] **Step 2: Run tests to verify failure**

```bash
cargo test -p vcli-ipc --lib submit_roundtrips_with_program_payload
cargo test -p vcli-cli --test e2e submit_command_sends_file_parent_as_base_dir
```

Expected: the new assertions fail because `Submit` has no `base_dir`.

- [ ] **Step 3: Add `base_dir` to `RequestOp::Submit`**

Change the enum variant:

```rust
Submit {
    program: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_dir: Option<String>,
}
```

Update all `RequestOp::Submit { program }` constructions to include `base_dir: None` except CLI submit.

- [ ] **Step 4: Send the canonical file parent from CLI**

In `commands::submit::run`, compute:

```rust
let base_dir = std::fs::canonicalize(&args.file)
    .ok()
    .and_then(|p| p.parent().map(|parent| parent.display().to_string()));
```

Then request:

```rust
RequestOp::Submit { program, base_dir }
```

- [ ] **Step 5: Verify and commit**

```bash
cargo test -p vcli-ipc --lib submit_roundtrips_with_program_payload
cargo test -p vcli-cli --test e2e submit_command_sends_file_parent_as_base_dir
cargo fmt --all -- --check
git add crates/vcli-ipc/src/wire/request.rs crates/vcli-ipc/tests/roundtrip.rs crates/vcli-cli/src/commands/submit.rs crates/vcli-cli/tests/e2e.rs docs/superpowers/plans/2026-05-01-vcli-template-assets.md
git commit -m "vcli-ipc: carry submit base_dir for asset resolution"
```

## Task 2: daemon ingests and rewrites template assets

**Files:**
- Create: `crates/vcli-daemon/src/assets.rs`
- Modify: `crates/vcli-daemon/src/lib.rs`
- Modify: `crates/vcli-daemon/src/handler.rs`

- [ ] **Step 1: Write failing helper tests**

Cover:
- relative `template.image` resolves under `base_dir`, writes a store asset, links `program_assets`, rewrites to `sha256:<hash>`, and returns `assets[hash] = bytes`;
- absolute paths work without `base_dir`;
- missing relative paths return `invalid_program`-class errors.

- [ ] **Step 2: Implement `materialize_template_assets`**

Create a daemon-only helper that recursively visits named predicates and inline watch predicates in a mutable `Program`, ingests every non-`sha256:` `Template.image`, rewrites the image field, links the asset to the program, and returns a `BTreeMap<String, Vec<u8>>` keyed by hash hex.

- [ ] **Step 3: Wire submit and resume**

During submit, validate raw JSON, materialize assets against `base_dir`, canonicalize the rewritten `Program`, insert the row, and send the rewritten program plus assets to the scheduler. During resume, reload the stored rewritten program and load its `sha256:` template assets from the store before sending `ResumeRunning`.

- [ ] **Step 4: Verify and commit**

```bash
cargo test -p vcli-daemon --lib assets
cargo test -p vcli-daemon --lib submit_rewrites_template_assets
cargo fmt --all -- --check
git add crates/vcli-daemon/src/assets.rs crates/vcli-daemon/src/lib.rs crates/vcli-daemon/src/handler.rs
git commit -m "vcli-daemon: materialize template assets on submit"
```

## Task 3: runtime evaluates templates with materialized bytes

**Files:**
- Modify: `crates/vcli-runtime/src/command.rs`
- Modify: `crates/vcli-runtime/src/program.rs`
- Modify: `crates/vcli-runtime/src/scheduler.rs`
- Modify runtime scenario helpers and daemon tests to include empty asset maps

- [ ] **Step 1: Write failing runtime scenario**

Add a scenario that submits a program with a `template` predicate whose image is `sha256:<hash>` and provides an asset map containing the PNG bytes. The scripted frame contains that template. One tick should emit `watch.fired` or complete a `wait_for` body.

- [ ] **Step 2: Add assets to scheduler commands and running programs**

Add `assets: BTreeMap<String, Vec<u8>>` to `SubmitValidated` and `ResumeRunning`; store it in `RunningProgram`; pass `&rp.assets` to triggers, watches, and body execution.

- [ ] **Step 3: Verify and commit**

```bash
cargo test -p vcli-runtime --test template_asset_runtime
cargo test -p vcli-daemon --lib
cargo fmt --all -- --check
git add crates/vcli-runtime/src/command.rs crates/vcli-runtime/src/program.rs crates/vcli-runtime/src/scheduler.rs crates/vcli-runtime/tests/scenarios/template_asset_runtime.rs crates/vcli-daemon/src/handler.rs
git commit -m "vcli-runtime: evaluate template predicates with program assets"
```

## Final gate

Run before the last push:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```
