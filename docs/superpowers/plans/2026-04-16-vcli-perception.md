# vcli-perception Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tier-1 and Tier-2 predicate evaluators with a DashMap-backed per-tick cache and cross-tick PerceptionState for stateful predicates.

**Architecture:** An `Evaluator` trait with `&self` eval (Decision A — safe under `rayon::par_iter`) is implemented by one struct per `PredicateKind`. A top-level `Perception` façade holds the shared `DashMap<PredicateHash, PredicateResult>` cache (per-tick; `clear()` called by the runtime at tick start) and a `PerceptionState` for cross-tick state (prior-frame snapshots for `pixel_diff`, first-true timestamps for `elapsed_ms_since_true`). Template matching uses **imageproc** (pure-Rust, portable, builds in CI without system deps); spec §"Template matching" and Decision 4.2 both name `imageproc` as the default. `opencv-rust` is a documented fallback if perf becomes a problem post-v0 — not in scope here.

**Tech Stack:** Rust (stable, 2021 edition), `vcli-core` (shared types), `dashmap` (lock-free cache), `imageproc` (template matching NCC), `image` (decode embedded PNG test fixtures), `thiserror` (error enum); dev-deps `proptest`, `approx`. No `tokio`, no `rayon` inside this crate — the runtime crate drives parallelism over our `&self` eval.

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md`. When this plan references a decision by number (e.g. "Decision A"), see the "Review decisions — 2026-04-16" appendix in that file.

**Depends on:** `vcli-core` (Phase 1 complete). This plan does NOT modify `vcli-core`.

---

## File structure produced by this plan

```
vcli/
├── Cargo.toml                                     # modify: add vcli-perception + workspace deps
└── crates/
    └── vcli-perception/
        ├── Cargo.toml
        ├── src/
        │   ├── lib.rs                             # module tree + re-exports
        │   ├── error.rs                           # PerceptionError + Result alias
        │   ├── evaluator.rs                       # Evaluator trait, EvalCtx
        │   ├── cache.rs                           # PredicateCache (DashMap)
        │   ├── state.rs                           # PerceptionState (cross-tick)
        │   ├── color_at.rs                        # ColorAtEvaluator
        │   ├── pixel_diff.rs                      # PixelDiffEvaluator + dHash helper
        │   ├── logical.rs                         # AllOf / AnyOf / Not evaluators
        │   ├── elapsed.rs                         # ElapsedMsSinceTrueEvaluator
        │   ├── template.rs                        # TemplateEvaluator + imageproc bridge
        │   ├── frame_view.rs                      # Frame → ImageBuffer borrow helpers
        │   └── perception.rs                      # Perception façade + evaluate_named
        └── tests/
            └── fixtures/
                ├── README.md                      # how the fixtures were generated
                ├── red_dot_200x200.png            # color_at / pixel_diff fixtures
                ├── blue_dot_200x200.png
                └── skip_button_40x16.png          # template target fixture
```

**Responsibility split rationale:** one evaluator per file, all under ~200 lines. `cache.rs` and `state.rs` are separate because they have distinct lifetimes (per-tick vs cross-tick). `frame_view.rs` isolates the BGRA-vs-RGBA swizzle and Arc<[u8]> → `image::ImageBuffer` bridge so every evaluator uses the same conversion.

---

## Tasks

### Task 1: Add workspace dependencies for perception crate

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Confirm current workspace state**

Run: `cat Cargo.toml`
Expected: workspace lists only `crates/vcli-core` and has `serde`, `serde_json`, `thiserror`, `uuid`, `ryu`, `unicode-normalization`, `proptest` as shared deps.

- [ ] **Step 2: Add vcli-perception shared deps to workspace `Cargo.toml`**

Modify the `[workspace]` and `[workspace.dependencies]` sections. Add `crates/vcli-perception` to members and append the new shared deps:

```toml
[workspace]
resolver = "2"
members = [
    "crates/vcli-core",
    "crates/vcli-perception",
]

# ... existing workspace.package ...

[workspace.dependencies]
# ... existing ...
dashmap = "6"
imageproc = "0.25"
image = { version = "0.25", default-features = false, features = ["png"] }

# Dev-only
# ... existing proptest = "1" ...
approx = "0.5"
```

Rationale: `image` is restricted to `png` (no jpeg / exr / webp) to keep build cost predictable in CI. `imageproc` 0.25 pairs with `image` 0.25.

- [ ] **Step 3: Verify workspace still parses (before the crate exists, this fails — that's fine)**

Run: `cargo check --workspace`
Expected: fails with "workspace member `crates/vcli-perception` does not exist". Accept; Task 2 creates it.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "vcli-perception: reserve workspace member + add imageproc/dashmap deps"
```

---

### Task 2: Scaffold the empty `vcli-perception` crate

**Files:**
- Create: `crates/vcli-perception/Cargo.toml`
- Create: `crates/vcli-perception/src/lib.rs`

- [ ] **Step 1: Verify parent directory exists**

Run: `ls crates/`
Expected: `vcli-core` listed; no `vcli-perception` yet.

- [ ] **Step 2: Write `crates/vcli-perception/Cargo.toml`**

```toml
[package]
name = "vcli-perception"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Tier-1/Tier-2 predicate evaluators + per-tick cache + cross-tick state for vcli."

[dependencies]
vcli-core = { path = "../vcli-core" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
dashmap = { workspace = true }
imageproc = { workspace = true }
image = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
approx = { workspace = true }
```

- [ ] **Step 3: Write minimal `crates/vcli-perception/src/lib.rs`**

```rust
//! vcli-perception — Tier-1 and Tier-2 predicate evaluators for vcli.
//!
//! See `docs/superpowers/specs/2026-04-16-vcli-design.md` §"Perception pipeline"
//! and Decision A (DashMap cache, `&self` eval under `par_iter`).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p vcli-perception`
Expected: clean check (may warn about empty lib).

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-perception/Cargo.toml crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: empty crate shell with core+imageproc+dashmap deps"
```

---

### Task 3: `PerceptionError` enum

**Files:**
- Create: `crates/vcli-perception/src/error.rs`
- Modify: `crates/vcli-perception/src/lib.rs`

- [ ] **Step 1: Add `pub mod error;` and re-export to `lib.rs`**

Append to `crates/vcli-perception/src/lib.rs`:

```rust
pub mod error;

pub use error::{PerceptionError, Result};
```

- [ ] **Step 2: Write tests + impl in `error.rs`**

```rust
//! Perception error type. Evaluators surface a fast-path `PredicateResult`
//! for "predicate is false because inputs disagree" (not an error), and
//! return `PerceptionError` only for conditions the runtime must handle
//! (bad asset bytes, out-of-bounds region, unknown referenced predicate).

use thiserror::Error;

/// Alias for `Result<T, PerceptionError>`.
pub type Result<T> = std::result::Result<T, PerceptionError>;

/// Errors surfaced by evaluators.
#[derive(Debug, Error)]
pub enum PerceptionError {
    /// A logical predicate referenced a name not present in the program.
    #[error("unknown predicate reference: {0}")]
    UnknownPredicate(String),
    /// A predicate referenced itself directly or transitively. The DSL
    /// validator should catch this at submit; the evaluator double-checks.
    #[error("cycle detected at predicate: {0}")]
    Cycle(String),
    /// Asset bytes could not be decoded.
    #[error("asset decode: {0}")]
    AssetDecode(String),
    /// Region is outside the frame bounds (partial overlap is clipped, but
    /// zero overlap is reported).
    #[error("region outside frame bounds")]
    RegionOutOfBounds,
    /// A `sha256:<hex>` asset reference was passed to an evaluator without
    /// a resolver having first attached the raw bytes. Bug, not user error.
    #[error("asset bytes not materialized for reference: {0}")]
    AssetNotMaterialized(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_unknown_predicate() {
        let e = PerceptionError::UnknownPredicate("foo".into());
        assert_eq!(e.to_string(), "unknown predicate reference: foo");
    }

    #[test]
    fn display_cycle() {
        let e = PerceptionError::Cycle("a".into());
        assert_eq!(e.to_string(), "cycle detected at predicate: a");
    }

    #[test]
    fn display_asset_decode() {
        let e = PerceptionError::AssetDecode("bad PNG".into());
        assert!(e.to_string().contains("bad PNG"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-perception --lib error`
Expected: 3 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-perception/src/error.rs crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: PerceptionError + Result alias"
```

---

### Task 4: `Evaluator` trait + `EvalCtx`

**Files:**
- Create: `crates/vcli-perception/src/evaluator.rs`
- Modify: `crates/vcli-perception/src/lib.rs`

- [ ] **Step 1: Add `pub mod evaluator;` and re-export**

Append to `lib.rs`:

```rust
pub mod evaluator;

pub use evaluator::{EvalCtx, Evaluator};
```

- [ ] **Step 2: Write `evaluator.rs`**

```rust
//! The `Evaluator` trait and its evaluation context.
//!
//! Per Decision A, `evaluate` takes `&self` so multiple evaluators can run
//! concurrently under `rayon::par_iter` with a shared `DashMap` cache.
//! Evaluators that need mutable state (pixel_diff prior frames, elapsed
//! first-true timestamps) push that state into `PerceptionState`, which
//! uses interior mutability (`DashMap` / `Mutex`) — never `&mut self`.

use std::collections::BTreeMap;

use vcli_core::clock::UnixMs;
use vcli_core::{Frame, Predicate, PredicateResult};

use crate::cache::PredicateCache;
use crate::error::Result;
use crate::state::PerceptionState;

/// Evaluation context threaded to every `Evaluator::evaluate` call.
///
/// Lifetime: exactly one tick. The cache is per-tick; the state is
/// cross-tick but shared by reference here.
pub struct EvalCtx<'a> {
    /// Captured frame for this tick.
    pub frame: &'a Frame,
    /// Unix-ms timestamp of this tick (from the `Clock`).
    pub now_ms: UnixMs,
    /// Per-tick cache (cleared by the runtime at tick start).
    pub cache: &'a PredicateCache,
    /// Cross-tick state (interior mutability).
    pub state: &'a PerceptionState,
    /// Named predicate graph for this program. Logical and
    /// elapsed_ms_since_true evaluators look up dependencies here.
    pub predicates: &'a BTreeMap<String, Predicate>,
    /// Asset bytes, keyed by sha256 hex (no `sha256:` prefix). Populated
    /// by the daemon submit module before handing off to Perception.
    pub assets: &'a BTreeMap<String, Vec<u8>>,
}

/// A predicate evaluator. One impl per predicate kind.
pub trait Evaluator: Send + Sync {
    /// Evaluate the predicate against the current frame + state.
    ///
    /// # Errors
    ///
    /// Returns `PerceptionError` only for conditions the runtime must
    /// handle (bad asset, region out of bounds, unknown reference).
    /// A "predicate is false because screen doesn't match" is NOT an
    /// error — it returns `Ok(PredicateResult { truthy: false, ... })`.
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time test: a no-op evaluator implements the trait.
    struct NoOp;
    impl Evaluator for NoOp {
        fn evaluate(&self, _p: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
            Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            })
        }
    }

    #[test]
    fn trait_is_object_safe_and_send_sync() {
        // Compiles only if Evaluator is object-safe and Send + Sync.
        let _boxed: Box<dyn Evaluator> = Box::new(NoOp);
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p vcli-perception`
Expected: fails — we reference `cache::PredicateCache` and `state::PerceptionState` which don't exist yet. That's intentional; we'll stub them in the next tasks to make compile succeed before proceeding.

- [ ] **Step 4: Create stub `cache.rs` and `state.rs` so the trait compiles**

Create `crates/vcli-perception/src/cache.rs`:

```rust
//! (stub — filled in by Task 5)
#![allow(dead_code, missing_docs)]
pub struct PredicateCache;
```

Create `crates/vcli-perception/src/state.rs`:

```rust
//! (stub — filled in by Task 6)
#![allow(dead_code, missing_docs)]
pub struct PerceptionState;
```

Append to `lib.rs`:

```rust
pub mod cache;
pub mod state;
```

- [ ] **Step 5: Run the evaluator tests**

Run: `cargo test -p vcli-perception --lib evaluator`
Expected: 1 test passing.

- [ ] **Step 6: Commit**

```bash
git add crates/vcli-perception/src/evaluator.rs crates/vcli-perception/src/cache.rs \
        crates/vcli-perception/src/state.rs crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: Evaluator trait + EvalCtx (object-safe, &self)"
```

---

### Task 5: `PredicateCache` (DashMap, per-tick)

**Files:**
- Modify: `crates/vcli-perception/src/cache.rs`

- [ ] **Step 1: Write failing tests first**

Replace `crates/vcli-perception/src/cache.rs` with the test-first scaffold:

```rust
//! Per-tick predicate result cache. Decision A — `DashMap` for lock-free
//! reads and sharded writes. The runtime calls `clear()` at the start of
//! every tick so evaluators within a tick see each other's results but
//! no stale results carry across ticks.
//!
//! Program-local temporal predicates (`elapsed_ms_since_true`) keep their
//! state in `PerceptionState`, not here.

use dashmap::DashMap;
use vcli_core::{PredicateHash, PredicateResult};

/// The shared per-tick cache.
#[derive(Debug, Default)]
pub struct PredicateCache {
    entries: DashMap<PredicateHash, PredicateResult>,
}

impl PredicateCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a cached result if present.
    #[must_use]
    pub fn get(&self, hash: &PredicateHash) -> Option<PredicateResult> {
        self.entries.get(hash).map(|r| r.clone())
    }

    /// Store a result. If an entry already exists for this hash, it is
    /// overwritten (this is the cheap retry path — it should be rare
    /// because evaluators check `get` first).
    pub fn insert(&self, hash: PredicateHash, result: PredicateResult) {
        self.entries.insert(hash, result);
    }

    /// Number of entries. Diagnostic use only.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Invalidate all entries. Called by the runtime at tick start.
    pub fn clear(&self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::predicate_hash;
    use serde_json::json;

    fn fake_result(truthy: bool) -> PredicateResult {
        PredicateResult {
            truthy,
            match_data: None,
            at: 1000,
        }
    }

    fn some_hash(tag: &str) -> PredicateHash {
        predicate_hash(&json!({"kind": "color_at", "tag": tag})).unwrap()
    }

    #[test]
    fn get_miss_returns_none() {
        let c = PredicateCache::new();
        assert!(c.get(&some_hash("a")).is_none());
        assert!(c.is_empty());
    }

    #[test]
    fn insert_then_get_returns_result() {
        let c = PredicateCache::new();
        let h = some_hash("a");
        c.insert(h.clone(), fake_result(true));
        assert_eq!(c.get(&h), Some(fake_result(true)));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn insert_overwrites_existing_entry() {
        let c = PredicateCache::new();
        let h = some_hash("a");
        c.insert(h.clone(), fake_result(false));
        c.insert(h.clone(), fake_result(true));
        assert_eq!(c.get(&h), Some(fake_result(true)));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn clear_wipes_all_entries() {
        let c = PredicateCache::new();
        c.insert(some_hash("a"), fake_result(true));
        c.insert(some_hash("b"), fake_result(false));
        assert_eq!(c.len(), 2);
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn concurrent_inserts_from_many_threads() {
        use std::sync::Arc;
        use std::thread;

        let c = Arc::new(PredicateCache::new());
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let c = Arc::clone(&c);
                thread::spawn(move || {
                    let tag = format!("k{i}");
                    c.insert(some_hash(&tag), fake_result(i % 2 == 0));
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(c.len(), 16);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p vcli-perception --lib cache`
Expected: 5 tests passing.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-perception/src/cache.rs
git commit -m "vcli-perception: PredicateCache (DashMap, per-tick clear)"
```

---

### Task 6: `PerceptionState` (cross-tick state)

**Files:**
- Modify: `crates/vcli-perception/src/state.rs`

- [ ] **Step 1: Write failing tests + impl**

Replace `crates/vcli-perception/src/state.rs`:

```rust
//! Cross-tick perception state. Distinct from `PredicateCache` which is
//! per-tick. `PerceptionState` carries:
//!
//! - prior-frame snapshots keyed by predicate hash, for `pixel_diff`
//! - first-true timestamps keyed by (program_id, predicate_name) pair,
//!   for `elapsed_ms_since_true`
//!
//! Interior mutability only — every public method takes `&self`.

use std::sync::Arc;

use dashmap::DashMap;

use vcli_core::clock::UnixMs;
use vcli_core::{PredicateHash, ProgramId};

/// A small perceptual summary of a region from a prior tick. The actual
/// representation is a `Vec<u8>` of 64-bit dHash bytes (8 bytes per
/// snapshot), not the raw pixels — we never store frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorSnapshot {
    /// 64-bit dHash, big-endian.
    pub dhash: u64,
    /// Tick at which this snapshot was recorded.
    pub at_ms: UnixMs,
}

/// Key for per-program-predicate transition tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProgramPredKey {
    /// Program that owns this transition timer.
    pub program: ProgramId,
    /// Predicate name within that program.
    pub predicate: String,
}

/// Cross-tick state shared across evaluators.
#[derive(Debug, Default)]
pub struct PerceptionState {
    /// Prior-frame dHashes by predicate hash (global — same cache key as
    /// the result cache, so cross-program dedup applies).
    prior_snapshots: DashMap<PredicateHash, PriorSnapshot>,
    /// First-true timestamps for `elapsed_ms_since_true`. Program-local
    /// per spec §"`elapsed_ms_since_true`".
    first_true_at: DashMap<ProgramPredKey, UnixMs>,
}

impl PerceptionState {
    /// Fresh empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the most recent snapshot for a pixel_diff predicate hash.
    #[must_use]
    pub fn prior_snapshot(&self, hash: &PredicateHash) -> Option<PriorSnapshot> {
        self.prior_snapshots.get(hash).map(|s| s.clone())
    }

    /// Record a new snapshot. Overwrites any existing entry for this key.
    pub fn record_snapshot(&self, hash: PredicateHash, snapshot: PriorSnapshot) {
        self.prior_snapshots.insert(hash, snapshot);
    }

    /// Read the first-true timestamp for a (program, predicate) pair, if
    /// the child predicate has been truthy on a prior tick without a
    /// falling edge in between.
    #[must_use]
    pub fn first_true_at(&self, key: &ProgramPredKey) -> Option<UnixMs> {
        self.first_true_at.get(key).map(|v| *v)
    }

    /// Record the edge into `true`. If the child was already true, this
    /// leaves the existing timestamp alone (caller should only call on a
    /// rising edge).
    pub fn set_first_true_at(&self, key: ProgramPredKey, at_ms: UnixMs) {
        self.first_true_at.entry(key).or_insert(at_ms);
    }

    /// Clear the first-true timestamp on a falling edge.
    pub fn clear_first_true_at(&self, key: &ProgramPredKey) {
        self.first_true_at.remove(key);
    }

    /// Convenience for the runtime to package `Arc<Self>` into `EvalCtx`.
    #[must_use]
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::predicate_hash;
    use serde_json::json;

    fn hash(tag: &str) -> PredicateHash {
        predicate_hash(&json!({"tag": tag})).unwrap()
    }

    fn ppk(program_id_str: &str, name: &str) -> ProgramPredKey {
        ProgramPredKey {
            program: program_id_str.parse().unwrap(),
            predicate: name.into(),
        }
    }

    #[test]
    fn prior_snapshot_round_trip() {
        let s = PerceptionState::new();
        let h = hash("a");
        assert!(s.prior_snapshot(&h).is_none());
        s.record_snapshot(
            h.clone(),
            PriorSnapshot {
                dhash: 0xDEAD_BEEF,
                at_ms: 1000,
            },
        );
        assert_eq!(
            s.prior_snapshot(&h),
            Some(PriorSnapshot {
                dhash: 0xDEAD_BEEF,
                at_ms: 1000,
            })
        );
    }

    #[test]
    fn record_snapshot_overwrites() {
        let s = PerceptionState::new();
        let h = hash("a");
        s.record_snapshot(
            h.clone(),
            PriorSnapshot {
                dhash: 1,
                at_ms: 100,
            },
        );
        s.record_snapshot(
            h.clone(),
            PriorSnapshot {
                dhash: 2,
                at_ms: 200,
            },
        );
        assert_eq!(s.prior_snapshot(&h).unwrap().dhash, 2);
    }

    #[test]
    fn first_true_set_then_read() {
        let s = PerceptionState::new();
        let k = ppk("00000000-0000-4000-8000-000000000001", "p");
        assert!(s.first_true_at(&k).is_none());
        s.set_first_true_at(k.clone(), 5000);
        assert_eq!(s.first_true_at(&k), Some(5000));
    }

    #[test]
    fn first_true_set_preserves_earliest() {
        let s = PerceptionState::new();
        let k = ppk("00000000-0000-4000-8000-000000000001", "p");
        s.set_first_true_at(k.clone(), 5000);
        s.set_first_true_at(k.clone(), 9000);
        assert_eq!(s.first_true_at(&k), Some(5000));
    }

    #[test]
    fn first_true_cleared_on_falling_edge() {
        let s = PerceptionState::new();
        let k = ppk("00000000-0000-4000-8000-000000000001", "p");
        s.set_first_true_at(k.clone(), 5000);
        s.clear_first_true_at(&k);
        assert!(s.first_true_at(&k).is_none());
    }

    #[test]
    fn different_programs_do_not_share_first_true_timestamps() {
        let s = PerceptionState::new();
        let a = ppk("00000000-0000-4000-8000-000000000001", "p");
        let b = ppk("00000000-0000-4000-8000-000000000002", "p");
        s.set_first_true_at(a.clone(), 1000);
        assert!(s.first_true_at(&b).is_none());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p vcli-perception --lib state`
Expected: 6 tests passing.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-perception/src/state.rs
git commit -m "vcli-perception: PerceptionState (prior snapshots + first-true timers)"
```

---

### Task 7: `frame_view` — Frame pixel access helpers

**Files:**
- Create: `crates/vcli-perception/src/frame_view.rs`
- Modify: `crates/vcli-perception/src/lib.rs`

- [ ] **Step 1: Add module**

Append to `lib.rs`:

```rust
pub mod frame_view;
```

- [ ] **Step 2: Write tests + impl in `frame_view.rs`**

```rust
//! Low-level accessors that bridge `vcli-core::Frame` (BGRA8 / RGBA8,
//! possibly with stride padding) to `image::RgbImage` slices used by
//! `imageproc` and the pixel_diff dHash.
//!
//! All evaluators go through this module so the BGRA↔RGB swizzle and the
//! Frame-bounds-vs-region clipping logic lives in exactly one place.

use image::{ImageBuffer, Rgb, RgbImage};

use vcli_core::geom::Rect;
use vcli_core::{Frame, FrameFormat};

use crate::error::{PerceptionError, Result};

/// Read the RGB value at `(x, y)` in frame-local (not screen) coords.
///
/// # Errors
///
/// Returns `RegionOutOfBounds` if the coordinate is outside the frame.
pub fn pixel_rgb(frame: &Frame, x: i32, y: i32) -> Result<[u8; 3]> {
    if x < 0 || y < 0 || x >= frame.width() || y >= frame.height() {
        return Err(PerceptionError::RegionOutOfBounds);
    }
    let ux = x as usize;
    let uy = y as usize;
    let bpp = frame.format.bytes_per_pixel();
    let offset = uy.saturating_mul(frame.stride) + ux.saturating_mul(bpp);
    let bytes = &frame.pixels[offset..offset + bpp];
    Ok(match frame.format {
        // BGRA8: B, G, R, A
        FrameFormat::Bgra8 => [bytes[2], bytes[1], bytes[0]],
        // RGBA8: R, G, B, A
        FrameFormat::Rgba8 => [bytes[0], bytes[1], bytes[2]],
    })
}

/// Crop a rectangle out of the frame and return it as an `RgbImage`.
/// `region_abs` is in absolute screen coords; this function translates
/// them into frame-local coords using `frame.bounds.top_left()`.
///
/// If the region partially overlaps, returns the overlap. If there is
/// zero overlap, returns `RegionOutOfBounds`.
///
/// # Errors
///
/// `RegionOutOfBounds` if the region does not intersect the frame.
pub fn crop_rgb(frame: &Frame, region_abs: Rect) -> Result<RgbImage> {
    let fb = frame.bounds;
    let x0 = region_abs.x.max(fb.x);
    let y0 = region_abs.y.max(fb.y);
    let x1 = (region_abs.x + region_abs.w).min(fb.x + fb.w);
    let y1 = (region_abs.y + region_abs.h).min(fb.y + fb.h);
    if x1 <= x0 || y1 <= y0 {
        return Err(PerceptionError::RegionOutOfBounds);
    }
    let w = (x1 - x0) as u32;
    let h = (y1 - y0) as u32;
    let mut out = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(w, h);
    for row in 0..h {
        for col in 0..w {
            let fx = (x0 - fb.x) + col as i32;
            let fy = (y0 - fb.y) + row as i32;
            let rgb = pixel_rgb(frame, fx, fy)?;
            out.put_pixel(col, row, Rgb(rgb));
        }
    }
    Ok(out)
}

/// Convert the full frame into an owned `RgbImage`. Used by
/// `TemplateEvaluator` when the region covers the whole frame.
///
/// # Errors
///
/// Propagates `pixel_rgb` errors (unreachable for in-bounds iteration).
pub fn frame_to_rgb(frame: &Frame) -> Result<RgbImage> {
    let w = frame.width() as u32;
    let h = frame.height() as u32;
    let mut out = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(w, h);
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            let rgb = pixel_rgb(frame, x, y)?;
            out.put_pixel(x as u32, y as u32, Rgb(rgb));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Build a 4×2 BGRA8 frame where pixel (x, y) = RGB (10·x, 20·y, x + y).
    fn bgra_test_frame() -> Frame {
        let w = 4usize;
        let h = 2usize;
        let stride = w * 4;
        let mut pixels = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let off = y * stride + x * 4;
                pixels[off] = (x + y) as u8; // B
                pixels[off + 1] = (20 * y) as u8; // G
                pixels[off + 2] = (10 * x) as u8; // R
                pixels[off + 3] = 255;
            }
        }
        Frame::new(
            FrameFormat::Bgra8,
            Rect {
                x: 0,
                y: 0,
                w: w as i32,
                h: h as i32,
            },
            stride,
            Arc::from(pixels),
            0,
        )
    }

    #[test]
    fn pixel_rgb_bgra_swizzles_to_rgb() {
        let f = bgra_test_frame();
        assert_eq!(pixel_rgb(&f, 0, 0).unwrap(), [0, 0, 0]);
        assert_eq!(pixel_rgb(&f, 3, 1).unwrap(), [30, 20, 4]);
    }

    #[test]
    fn pixel_rgb_out_of_bounds_errors() {
        let f = bgra_test_frame();
        assert!(matches!(
            pixel_rgb(&f, -1, 0),
            Err(PerceptionError::RegionOutOfBounds)
        ));
        assert!(matches!(
            pixel_rgb(&f, 0, 10),
            Err(PerceptionError::RegionOutOfBounds)
        ));
    }

    #[test]
    fn crop_rgb_returns_correct_dimensions() {
        let f = bgra_test_frame();
        let crop = crop_rgb(
            &f,
            Rect {
                x: 1,
                y: 0,
                w: 2,
                h: 2,
            },
        )
        .unwrap();
        assert_eq!(crop.width(), 2);
        assert_eq!(crop.height(), 2);
        // (1, 0) = R 10, G 0, B 1
        assert_eq!(crop.get_pixel(0, 0).0, [10, 0, 1]);
    }

    #[test]
    fn crop_rgb_clips_to_frame_bounds() {
        let f = bgra_test_frame();
        let crop = crop_rgb(
            &f,
            Rect {
                x: 3,
                y: 1,
                w: 10,
                h: 10,
            },
        )
        .unwrap();
        assert_eq!(crop.width(), 1);
        assert_eq!(crop.height(), 1);
    }

    #[test]
    fn crop_rgb_zero_overlap_errors() {
        let f = bgra_test_frame();
        let crop = crop_rgb(
            &f,
            Rect {
                x: 100,
                y: 100,
                w: 10,
                h: 10,
            },
        );
        assert!(matches!(crop, Err(PerceptionError::RegionOutOfBounds)));
    }

    #[test]
    fn frame_to_rgb_has_correct_dimensions() {
        let f = bgra_test_frame();
        let img = frame_to_rgb(&f).unwrap();
        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 2);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-perception --lib frame_view`
Expected: 6 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-perception/src/frame_view.rs crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: frame_view (BGRA↔RGB swizzle, region crop with clipping)"
```

---

### Task 8: `ColorAtEvaluator` (Tier 1)

**Files:**
- Create: `crates/vcli-perception/src/color_at.rs`
- Modify: `crates/vcli-perception/src/lib.rs`

- [ ] **Step 1: Add module + re-export**

Append to `lib.rs`:

```rust
pub mod color_at;

pub use color_at::ColorAtEvaluator;
```

- [ ] **Step 2: Write tests + impl in `color_at.rs`**

```rust
//! `ColorAtEvaluator` — Tier 1, <1ms. Samples one pixel and checks its
//! Euclidean RGB distance from the target.

use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::{Frame, Predicate, PredicateResult};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::frame_view::pixel_rgb;

/// Stateless evaluator for `PredicateKind::ColorAt`.
#[derive(Debug, Default)]
pub struct ColorAtEvaluator;

impl ColorAtEvaluator {
    /// Pure sampling helper. Public for tests; prefer `Evaluator::evaluate`.
    ///
    /// # Errors
    ///
    /// Propagates `RegionOutOfBounds` if the point is outside the frame.
    pub fn sample(frame: &Frame, x: i32, y: i32, target: Rgb, tolerance: u16) -> Result<bool> {
        let [r, g, b] = pixel_rgb(frame, x, y)?;
        let [tr, tg, tb] = target.0;
        let dr = i32::from(r) - i32::from(tr);
        let dg = i32::from(g) - i32::from(tg);
        let db = i32::from(b) - i32::from(tb);
        let dist_sq = dr * dr + dg * dg + db * db;
        let tol_sq = i32::from(tolerance) * i32::from(tolerance);
        Ok(dist_sq <= tol_sq)
    }
}

impl Evaluator for ColorAtEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::ColorAt {
            point,
            rgb,
            tolerance,
        } = predicate
        else {
            return Err(PerceptionError::AssetDecode(
                "color_at evaluator received non-color_at predicate".into(),
            ));
        };
        // Translate absolute screen point to frame-local.
        let fb = ctx.frame.bounds;
        let fx = point.x - fb.x;
        let fy = point.y - fb.y;
        let truthy = Self::sample(ctx.frame, fx, fy, *rgb, *tolerance)?;
        Ok(PredicateResult {
            truthy,
            match_data: None,
            at: ctx.now_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::Rgb;
    use vcli_core::{FrameFormat, PredicateKind};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    /// 2×2 RGBA8 frame: (0,0)=red, (1,0)=green, (0,1)=blue, (1,1)=white.
    fn rgba_checker() -> Frame {
        let pixels: Vec<u8> = vec![
            255, 0, 0, 255, // (0,0) red
            0, 255, 0, 255, // (1,0) green
            0, 0, 255, 255, // (0,1) blue
            255, 255, 255, 255, // (1,1) white
        ];
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            },
            8,
            Arc::from(pixels),
            0,
        )
    }

    fn ctx<'a>(
        frame: &'a Frame,
        cache: &'a PredicateCache,
        state: &'a PerceptionState,
        preds: &'a BTreeMap<String, Predicate>,
        assets: &'a BTreeMap<String, Vec<u8>>,
    ) -> EvalCtx<'a> {
        EvalCtx {
            frame,
            now_ms: 42,
            cache,
            state,
            predicates: preds,
            assets,
        }
    }

    #[test]
    fn exact_color_match_is_truthy() {
        let f = rgba_checker();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([255, 0, 0]),
            tolerance: 0,
        };
        let r = ColorAtEvaluator
            .evaluate(&p, &ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(r.truthy);
        assert_eq!(r.at, 42);
        assert!(r.match_data.is_none());
    }

    #[test]
    fn off_by_one_within_tolerance_is_truthy() {
        let f = rgba_checker();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = PredicateKind::ColorAt {
            point: Point { x: 1, y: 0 },
            rgb: Rgb([1, 254, 1]),
            tolerance: 5,
        };
        let r = ColorAtEvaluator
            .evaluate(&p, &ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(r.truthy);
    }

    #[test]
    fn beyond_tolerance_is_falsy() {
        let f = rgba_checker();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = PredicateKind::ColorAt {
            point: Point { x: 0, y: 1 }, // blue
            rgb: Rgb([255, 0, 0]),       // red target
            tolerance: 10,
        };
        let r = ColorAtEvaluator
            .evaluate(&p, &ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn out_of_bounds_errors() {
        let f = rgba_checker();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let p = PredicateKind::ColorAt {
            point: Point { x: 100, y: 100 },
            rgb: Rgb([0, 0, 0]),
            tolerance: 0,
        };
        let r = ColorAtEvaluator.evaluate(&p, &ctx(&f, &cache, &state, &preds, &assets));
        assert!(matches!(r, Err(PerceptionError::RegionOutOfBounds)));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-perception --lib color_at`
Expected: 4 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-perception/src/color_at.rs crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: ColorAtEvaluator (Tier-1 pixel sample + RGB distance)"
```

---

### Task 9: `PixelDiffEvaluator` (Tier 1 with dHash + prior-frame state)

**Files:**
- Create: `crates/vcli-perception/src/pixel_diff.rs`
- Modify: `crates/vcli-perception/src/lib.rs`

- [ ] **Step 1: Add module + re-export**

Append to `lib.rs`:

```rust
pub mod pixel_diff;

pub use pixel_diff::PixelDiffEvaluator;
```

- [ ] **Step 2: Write tests + impl in `pixel_diff.rs`**

```rust
//! `PixelDiffEvaluator` — Tier 1. Perceptual-hash (dHash) over a region,
//! compared to a prior-tick hash stored in `PerceptionState`.
//!
//! Spec §"Color / pixel diff": dHash over the region + Hamming distance
//! against the baseline. In v0 the "baseline" is the previous tick's
//! snapshot of the same region (this is what gives us a cheap motion
//! detector). When no prior snapshot exists, the predicate is falsy and
//! we just record the baseline for next tick.

use image::imageops::FilterType;
use image::{imageops, GrayImage};
use serde_json::json;

use vcli_core::{canonicalize, Frame, Predicate, PredicateHash, PredicateResult};
use vcli_core::predicate::PredicateKind;

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::frame_view::crop_rgb;
use crate::state::PriorSnapshot;

/// Stateless evaluator for `PredicateKind::PixelDiff`. State lives in
/// `PerceptionState::prior_snapshots`.
#[derive(Debug, Default)]
pub struct PixelDiffEvaluator;

impl PixelDiffEvaluator {
    /// Compute a 64-bit dHash over a grayscale thumbnail (9×8 resample).
    /// Bit `i` is set iff pixel `(x, y)` is brighter than `(x+1, y)` for
    /// the 8×8 block produced by taking the 9×8 differences.
    #[must_use]
    pub fn dhash(gray: &GrayImage) -> u64 {
        let resized = imageops::resize(gray, 9, 8, FilterType::Lanczos3);
        let mut h: u64 = 0;
        for y in 0..8u32 {
            for x in 0..8u32 {
                let l = resized.get_pixel(x, y).0[0];
                let r = resized.get_pixel(x + 1, y).0[0];
                h <<= 1;
                if l > r {
                    h |= 1;
                }
            }
        }
        h
    }

    /// Hamming distance between two 64-bit hashes.
    #[must_use]
    pub fn hamming(a: u64, b: u64) -> u32 {
        (a ^ b).count_ones()
    }

    /// Convenience: wrap an Rgb crop into a grayscale buffer.
    fn grayscale(img: &image::RgbImage) -> GrayImage {
        imageops::grayscale(img)
    }
}

impl Evaluator for PixelDiffEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::PixelDiff {
            region,
            baseline,
            threshold,
        } = predicate
        else {
            return Err(PerceptionError::AssetDecode(
                "pixel_diff evaluator received wrong predicate kind".into(),
            ));
        };

        // v0 scope: only Region::Absolute is handled directly here. Window
        // + RelativeTo are resolved by the runtime before calling us and
        // passed in as Absolute. If we see a non-absolute, treat as false
        // for v0 (the runtime should never hand us one).
        let rect = match region {
            vcli_core::Region::Absolute { rect } => *rect,
            _ => {
                return Ok(PredicateResult {
                    truthy: false,
                    match_data: None,
                    at: ctx.now_ms,
                });
            }
        };

        // Hash key for state lookup: predicate-local (baseline + region).
        let key_value = json!({
            "kind": "pixel_diff_state",
            "baseline": baseline,
            "rect": [rect.x, rect.y, rect.w, rect.h],
        });
        let key_bytes = canonicalize(&key_value).map_err(|e| {
            PerceptionError::AssetDecode(format!("canonicalize pixel_diff key: {e}"))
        })?;
        // SHA over canonical bytes via vcli_core::predicate_hash on the same value.
        let state_key = vcli_core::predicate_hash(&key_value).map_err(|e| {
            PerceptionError::AssetDecode(format!("hash pixel_diff key: {e}"))
        })?;
        let _ = key_bytes; // Retained for future diagnostics; canonicalize call asserts stability.

        // Compute current dHash of the region.
        let current_crop = crop_rgb(ctx.frame, rect)?;
        let current_gray = Self::grayscale(&current_crop);
        let current_hash = Self::dhash(&current_gray);

        let prior = ctx.state.prior_snapshot(&state_key);
        // Always record the current snapshot for the next tick.
        ctx.state.record_snapshot(
            state_key.clone(),
            PriorSnapshot {
                dhash: current_hash,
                at_ms: ctx.now_ms,
            },
        );

        // No prior snapshot → first tick, not truthy (nothing to diff).
        let Some(prev) = prior else {
            return Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            });
        };

        // Fraction-of-64 Hamming distance vs threshold.
        let distance = Self::hamming(current_hash, prev.dhash) as f64 / 64.0;
        let truthy = distance >= *threshold;
        Ok(PredicateResult {
            truthy,
            match_data: None,
            at: ctx.now_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use image::{Rgb, RgbImage};
    use vcli_core::geom::Rect;
    use vcli_core::predicate::PredicateKind;
    use vcli_core::{FrameFormat, Region};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    /// 32×32 solid RGBA frame filled with `color`.
    fn solid_frame(color: [u8; 3]) -> Frame {
        let w = 32usize;
        let h = 32usize;
        let stride = w * 4;
        let mut pixels = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let off = y * stride + x * 4;
                pixels[off] = color[0];
                pixels[off + 1] = color[1];
                pixels[off + 2] = color[2];
                pixels[off + 3] = 255;
            }
        }
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: w as i32,
                h: h as i32,
            },
            stride,
            Arc::from(pixels),
            0,
        )
    }

    #[test]
    fn dhash_and_hamming_on_identical_images_is_zero() {
        let img = RgbImage::from_pixel(32, 32, Rgb([128, 128, 128]));
        let g = PixelDiffEvaluator::grayscale(&img);
        let h1 = PixelDiffEvaluator::dhash(&g);
        let h2 = PixelDiffEvaluator::dhash(&g);
        assert_eq!(PixelDiffEvaluator::hamming(h1, h2), 0);
    }

    #[test]
    fn dhash_differs_for_contrasting_images() {
        let uniform = RgbImage::from_pixel(32, 32, Rgb([128, 128, 128]));
        let mut striped = RgbImage::from_pixel(32, 32, Rgb([0, 0, 0]));
        for y in 0..32u32 {
            for x in (0..32u32).step_by(2) {
                striped.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        let h1 = PixelDiffEvaluator::dhash(&PixelDiffEvaluator::grayscale(&uniform));
        let h2 = PixelDiffEvaluator::dhash(&PixelDiffEvaluator::grayscale(&striped));
        // Striping is dramatic — should flip most bits.
        assert!(PixelDiffEvaluator::hamming(h1, h2) > 16);
    }

    #[test]
    fn first_tick_not_truthy_but_records_snapshot() {
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let frame = solid_frame([200, 50, 50]);
        let p = PredicateKind::PixelDiff {
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 32,
                    h: 32,
                },
            },
            baseline: "sha256:unused".into(),
            threshold: 0.1,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
        };
        let r = PixelDiffEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn same_scene_on_second_tick_is_not_truthy() {
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let frame = solid_frame([200, 50, 50]);
        let p = PredicateKind::PixelDiff {
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 32,
                    h: 32,
                },
            },
            baseline: "sha256:unused".into(),
            threshold: 0.1,
        };
        // Tick 1: record snapshot.
        let ctx1 = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
        };
        let _ = PixelDiffEvaluator.evaluate(&p, &ctx1).unwrap();
        // Tick 2: same frame, no diff.
        let ctx2 = EvalCtx {
            frame: &frame,
            now_ms: 1100,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
        };
        let r = PixelDiffEvaluator.evaluate(&p, &ctx2).unwrap();
        assert!(!r.truthy, "identical frames should not diff");
    }

    #[test]
    fn changed_scene_on_second_tick_is_truthy() {
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();

        // Two frames that will produce very different dHashes.
        let frame_a = solid_frame([10, 10, 10]);
        // For B, alternate rows of black/white — drastically different gray.
        let mut frame_b_pixels = vec![0u8; 32 * 32 * 4];
        for y in 0..32usize {
            for x in 0..32usize {
                let off = y * 32 * 4 + x * 4;
                let v = if (x + y) % 2 == 0 { 0u8 } else { 255u8 };
                frame_b_pixels[off] = v;
                frame_b_pixels[off + 1] = v;
                frame_b_pixels[off + 2] = v;
                frame_b_pixels[off + 3] = 255;
            }
        }
        let frame_b = Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 32,
                h: 32,
            },
            32 * 4,
            Arc::from(frame_b_pixels),
            0,
        );

        let p = PredicateKind::PixelDiff {
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 32,
                    h: 32,
                },
            },
            baseline: "sha256:unused".into(),
            threshold: 0.1,
        };
        let ctx1 = EvalCtx {
            frame: &frame_a,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
        };
        let _ = PixelDiffEvaluator.evaluate(&p, &ctx1).unwrap();
        let ctx2 = EvalCtx {
            frame: &frame_b,
            now_ms: 1100,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
        };
        let r = PixelDiffEvaluator.evaluate(&p, &ctx2).unwrap();
        assert!(r.truthy);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-perception --lib pixel_diff`
Expected: 5 tests passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-perception/src/pixel_diff.rs crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: PixelDiffEvaluator (dHash + prior-frame snapshot state)"
```

---

### Task 10: Logical evaluators — `AllOfEvaluator`, `AnyOfEvaluator`, `NotEvaluator`

**Files:**
- Create: `crates/vcli-perception/src/logical.rs`
- Modify: `crates/vcli-perception/src/lib.rs`

- [ ] **Step 1: Add module + re-exports**

Append to `lib.rs`:

```rust
pub mod logical;

pub use logical::{AllOfEvaluator, AnyOfEvaluator, NotEvaluator};
```

- [ ] **Step 2: Write tests + impls in `logical.rs`**

```rust
//! Logical composition evaluators — Tier 1. `all_of`, `any_of`, `not`
//! recurse through named predicates in `EvalCtx::predicates`, threading
//! the shared cache so siblings sharing a dependency pay only once.
//!
//! These are the first evaluators that _call back_ into the perception
//! layer for their children. They use a depth-bounded recursion guard so
//! a malformed program (which the DSL validator should have caught at
//! submit) can't crash the daemon.

use std::collections::BTreeMap;

use vcli_core::predicate::PredicateKind;
use vcli_core::{predicate_hash, Predicate, PredicateHash, PredicateResult};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};

const MAX_RECURSION_DEPTH: u32 = 32;

/// Look up a named predicate, evaluate it through the correct kind's
/// evaluator, memoize in the cache, and return the result. Used by
/// logical + elapsed_ms_since_true evaluators.
///
/// # Errors
///
/// - `UnknownPredicate` if `name` is not in `ctx.predicates`.
/// - `Cycle` if recursion exceeds `MAX_RECURSION_DEPTH`.
/// - Any error the child evaluator produces.
pub(crate) fn evaluate_named_with_depth(
    name: &str,
    ctx: &EvalCtx<'_>,
    depth: u32,
) -> Result<PredicateResult> {
    if depth >= MAX_RECURSION_DEPTH {
        return Err(PerceptionError::Cycle(name.into()));
    }
    let pred = ctx
        .predicates
        .get(name)
        .ok_or_else(|| PerceptionError::UnknownPredicate(name.into()))?;

    let hash = hash_predicate(pred)?;
    if let Some(cached) = ctx.cache.get(&hash) {
        return Ok(cached);
    }

    let result = match pred {
        PredicateKind::AllOf { of } => eval_all_of(of, ctx, depth + 1)?,
        PredicateKind::AnyOf { of } => eval_any_of(of, ctx, depth + 1)?,
        PredicateKind::Not { of } => eval_not(of, ctx, depth + 1)?,
        PredicateKind::ElapsedMsSinceTrue { .. }
        | PredicateKind::ColorAt { .. }
        | PredicateKind::PixelDiff { .. }
        | PredicateKind::Template { .. } => {
            // Non-logical kinds are dispatched by the Perception façade.
            // `evaluate_named_with_depth` is only called from logical /
            // elapsed contexts; those callers dispatch through the
            // façade (see `perception.rs`), which handles all kinds.
            crate::perception::dispatch_leaf(pred, ctx)?
        }
    };

    ctx.cache.insert(hash, result.clone());
    Ok(result)
}

fn hash_predicate(p: &Predicate) -> Result<PredicateHash> {
    let v = serde_json::to_value(p).map_err(|e| {
        PerceptionError::AssetDecode(format!("serialize predicate for hash: {e}"))
    })?;
    predicate_hash(&v)
        .map_err(|e| PerceptionError::AssetDecode(format!("hash predicate: {e}")))
}

fn eval_all_of(of: &[String], ctx: &EvalCtx<'_>, depth: u32) -> Result<PredicateResult> {
    // Short-circuit at first falsy child.
    for name in of {
        let r = evaluate_named_with_depth(name, ctx, depth)?;
        if !r.truthy {
            return Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            });
        }
    }
    Ok(PredicateResult {
        truthy: true,
        match_data: None,
        at: ctx.now_ms,
    })
}

fn eval_any_of(of: &[String], ctx: &EvalCtx<'_>, depth: u32) -> Result<PredicateResult> {
    for name in of {
        let r = evaluate_named_with_depth(name, ctx, depth)?;
        if r.truthy {
            return Ok(PredicateResult {
                truthy: true,
                match_data: None,
                at: ctx.now_ms,
            });
        }
    }
    Ok(PredicateResult {
        truthy: false,
        match_data: None,
        at: ctx.now_ms,
    })
}

fn eval_not(name: &str, ctx: &EvalCtx<'_>, depth: u32) -> Result<PredicateResult> {
    let child = evaluate_named_with_depth(name, ctx, depth)?;
    Ok(PredicateResult {
        truthy: !child.truthy,
        match_data: None,
        at: ctx.now_ms,
    })
}

/// Public evaluator for `all_of`. Delegates to the shared traversal.
#[derive(Debug, Default)]
pub struct AllOfEvaluator;

impl Evaluator for AllOfEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::AllOf { of } = predicate else {
            return Err(PerceptionError::AssetDecode(
                "all_of evaluator got wrong kind".into(),
            ));
        };
        eval_all_of(of, ctx, 0)
    }
}

/// Public evaluator for `any_of`.
#[derive(Debug, Default)]
pub struct AnyOfEvaluator;

impl Evaluator for AnyOfEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::AnyOf { of } = predicate else {
            return Err(PerceptionError::AssetDecode(
                "any_of evaluator got wrong kind".into(),
            ));
        };
        eval_any_of(of, ctx, 0)
    }
}

/// Public evaluator for `not`.
#[derive(Debug, Default)]
pub struct NotEvaluator;

impl Evaluator for NotEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::Not { of } = predicate else {
            return Err(PerceptionError::AssetDecode(
                "not evaluator got wrong kind".into(),
            ));
        };
        eval_not(of, ctx, 0)
    }
}

/// Internal constructor — logical evaluators don't need this, but
/// `perception.rs` re-exports `evaluate_named` for external callers.
pub use evaluate_named_with_depth as crate_evaluate_named_with_depth;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::{Rgb};
    use vcli_core::{FrameFormat, Frame};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    fn red_pixel_frame() -> Frame {
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            4,
            Arc::from(pixels),
            0,
        )
    }

    fn build_preds() -> BTreeMap<String, Predicate> {
        let mut m = BTreeMap::new();
        m.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        m.insert(
            "green_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([0, 255, 0]),
                tolerance: 0,
            },
        );
        m
    }

    fn make_ctx<'a>(
        frame: &'a Frame,
        cache: &'a PredicateCache,
        state: &'a PerceptionState,
        preds: &'a BTreeMap<String, Predicate>,
        assets: &'a BTreeMap<String, Vec<u8>>,
    ) -> EvalCtx<'a> {
        EvalCtx {
            frame,
            now_ms: 10,
            cache,
            state,
            predicates: preds,
            assets,
        }
    }

    #[test]
    fn not_red_when_actually_red_is_false() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::Not {
            of: "red_here".into(),
        };
        let r = NotEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn all_of_true_when_all_children_true() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AllOf {
            of: vec!["red_here".into(), "red_here".into()],
        };
        let r = AllOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(r.truthy);
    }

    #[test]
    fn all_of_false_if_any_child_false() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AllOf {
            of: vec!["red_here".into(), "green_here".into()],
        };
        let r = AllOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn any_of_true_if_any_child_true() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AnyOf {
            of: vec!["green_here".into(), "red_here".into()],
        };
        let r = AnyOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(r.truthy);
    }

    #[test]
    fn any_of_empty_list_is_false() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AnyOf { of: vec![] };
        let r = AnyOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn unknown_named_predicate_errors() {
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::Not {
            of: "does_not_exist".into(),
        };
        let r = NotEvaluator.evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets));
        assert!(matches!(r, Err(PerceptionError::UnknownPredicate(_))));
    }

    #[test]
    fn logical_shares_cache_across_siblings() {
        // Evaluate (red_here AND red_here) — the second call should hit the
        // cache. We detect this by wrapping ColorAtEvaluator in a counter?
        // Simpler: assert the cache grows to at most 2 entries (red_here
        // result + the all_of result).
        let f = red_pixel_frame();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = build_preds();
        let assets = BTreeMap::new();
        let p = PredicateKind::AllOf {
            of: vec!["red_here".into(), "red_here".into()],
        };
        let _ = AllOfEvaluator
            .evaluate(&p, &make_ctx(&f, &cache, &state, &preds, &assets))
            .unwrap();
        // Cache should contain exactly one entry for `red_here`. The
        // `all_of` result is inserted by the Perception façade, not by the
        // logical evaluator itself.
        assert_eq!(cache.len(), 1, "red_here cached once, not twice");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-perception --lib logical`
Expected: 7 tests. They will fail to compile until Task 12 defines `crate::perception::dispatch_leaf`. That's intentional — we'll stub it now.

- [ ] **Step 4: Stub `perception.rs` so `dispatch_leaf` exists**

Create `crates/vcli-perception/src/perception.rs` with a stub:

```rust
//! (stub — filled in by Task 12)
#![allow(missing_docs)]

use vcli_core::{Predicate, PredicateResult};

use crate::error::{PerceptionError, Result};
use crate::evaluator::EvalCtx;

pub(crate) fn dispatch_leaf(_p: &Predicate, _ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
    Err(PerceptionError::AssetDecode("dispatch_leaf stub".into()))
}
```

Append to `lib.rs`:

```rust
pub mod perception;
```

- [ ] **Step 5: Re-run logical tests (should now compile and pass)**

Run: `cargo test -p vcli-perception --lib logical`
Expected: 7 tests passing.

- [ ] **Step 6: Commit**

```bash
git add crates/vcli-perception/src/logical.rs crates/vcli-perception/src/perception.rs \
        crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: AllOf / AnyOf / Not evaluators with shared-cache recursion"
```

---

### Task 11: `ElapsedMsSinceTrueEvaluator` (Tier 1, program-local state)

**Files:**
- Create: `crates/vcli-perception/src/elapsed.rs`
- Modify: `crates/vcli-perception/src/lib.rs`
- Modify: `crates/vcli-perception/src/evaluator.rs` — add optional `program` field to `EvalCtx`

- [ ] **Step 1: Extend `EvalCtx` with the current program id**

Per spec §"`elapsed_ms_since_true`", the timer is **program-local**. Modify `crates/vcli-perception/src/evaluator.rs` to add a `program` field:

```rust
// inside EvalCtx struct, add:
    /// The program currently being evaluated. Only used by predicates with
    /// program-local state (`elapsed_ms_since_true`). `None` when the
    /// runtime evaluates a trigger-independent predicate.
    pub program: Option<vcli_core::ProgramId>,
```

Also update the no-op test evaluator in that file to set `program: None` when constructing ctxs (they already don't construct one — but add the line to the doc-comment example if present).

- [ ] **Step 2: Update existing tests to set `program: None`**

Files: `color_at.rs`, `pixel_diff.rs`, `logical.rs`. Every `EvalCtx { … }` literal needs `program: None`. Add the field; confirm tests still compile.

Run: `cargo check -p vcli-perception --tests`
Expected: clean.

Run: `cargo test -p vcli-perception`
Expected: all prior tests pass.

- [ ] **Step 3: Add module + re-export**

Append to `lib.rs`:

```rust
pub mod elapsed;

pub use elapsed::ElapsedMsSinceTrueEvaluator;
```

- [ ] **Step 4: Write tests + impl in `elapsed.rs`**

```rust
//! `ElapsedMsSinceTrueEvaluator` — Tier 1. True iff the referenced child
//! predicate has been continuously truthy for at least `ms` milliseconds.
//!
//! State is program-local (spec §"`elapsed_ms_since_true`"): timestamps
//! live in `PerceptionState::first_true_at` keyed by
//! `(program_id, predicate_name)`, not by predicate hash. Without a
//! program id in `EvalCtx`, this evaluator returns an error (the runtime
//! should never call it in that configuration).

use vcli_core::predicate::PredicateKind;
use vcli_core::{Predicate, PredicateResult};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::logical::crate_evaluate_named_with_depth;
use crate::state::ProgramPredKey;

/// Evaluator for `PredicateKind::ElapsedMsSinceTrue`.
#[derive(Debug, Default)]
pub struct ElapsedMsSinceTrueEvaluator;

impl Evaluator for ElapsedMsSinceTrueEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::ElapsedMsSinceTrue {
            predicate: child_name,
            ms,
        } = predicate
        else {
            return Err(PerceptionError::AssetDecode(
                "elapsed evaluator got wrong kind".into(),
            ));
        };

        let program = ctx.program.ok_or_else(|| {
            PerceptionError::AssetDecode(
                "elapsed_ms_since_true evaluated outside program context".into(),
            )
        })?;

        let child = crate_evaluate_named_with_depth(child_name, ctx, 0)?;

        let key = ProgramPredKey {
            program,
            predicate: child_name.clone(),
        };

        if child.truthy {
            // Rising edge: record the first-true timestamp. No-op if already set.
            ctx.state.set_first_true_at(key.clone(), ctx.now_ms);
            // Now check whether we've been true long enough.
            let first = ctx.state.first_true_at(&key).unwrap_or(ctx.now_ms);
            let elapsed = ctx.now_ms.saturating_sub(first);
            let truthy = elapsed >= i64::from(*ms);
            Ok(PredicateResult {
                truthy,
                match_data: None,
                at: ctx.now_ms,
            })
        } else {
            // Falling edge: clear so the next true starts a new run.
            ctx.state.clear_first_true_at(&key);
            Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::Rgb;
    use vcli_core::{Frame, FrameFormat, ProgramId};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    fn red_pixel_frame() -> Frame {
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            4,
            Arc::from(vec![255u8, 0, 0, 255]),
            0,
        )
    }

    fn make_preds(child_truthy_color: Rgb) -> BTreeMap<String, Predicate> {
        let mut m = BTreeMap::new();
        m.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: child_truthy_color,
                tolerance: 0,
            },
        );
        m
    }

    #[test]
    fn first_tick_truthy_child_not_elapsed_yet() {
        let state = PerceptionState::new();
        let cache = PredicateCache::new();
        let preds = make_preds(Rgb([255, 0, 0]));
        let assets = BTreeMap::new();
        let frame = red_pixel_frame();
        let pid = ProgramId::new();
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "red_here".into(),
            ms: 500,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: Some(pid),
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy, "0ms elapsed is not ≥500");
    }

    #[test]
    fn after_threshold_elapses_returns_truthy() {
        let state = PerceptionState::new();
        let cache = PredicateCache::new();
        let preds = make_preds(Rgb([255, 0, 0]));
        let assets = BTreeMap::new();
        let frame = red_pixel_frame();
        let pid = ProgramId::new();
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "red_here".into(),
            ms: 500,
        };
        let ctx1 = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: Some(pid),
        };
        let _ = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx1).unwrap();
        cache.clear();
        let ctx2 = EvalCtx {
            frame: &frame,
            now_ms: 1500, // 500ms later, still truthy
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: Some(pid),
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx2).unwrap();
        assert!(r.truthy);
    }

    #[test]
    fn falling_edge_clears_timer() {
        let state = PerceptionState::new();
        let cache = PredicateCache::new();
        let preds_red = make_preds(Rgb([255, 0, 0]));
        let preds_green = make_preds(Rgb([0, 255, 0]));
        let assets = BTreeMap::new();
        let frame = red_pixel_frame();
        let pid = ProgramId::new();
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "red_here".into(),
            ms: 500,
        };

        // Tick 1: child truthy, 0ms elapsed.
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds_red,
            assets: &assets,
            program: Some(pid),
        };
        let _ = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx).unwrap();

        // Tick 2: child becomes FALSE (expect green, frame is red).
        cache.clear();
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1200,
            cache: &cache,
            state: &state,
            predicates: &preds_green,
            assets: &assets,
            program: Some(pid),
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy);

        // Tick 3: back to truthy at t=2000. Should re-start the timer —
        // elapsed is 0, not 1000.
        cache.clear();
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 2000,
            cache: &cache,
            state: &state,
            predicates: &preds_red,
            assets: &assets,
            program: Some(pid),
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy, "timer restarted after falling edge");
    }

    #[test]
    fn no_program_id_errors() {
        let state = PerceptionState::new();
        let cache = PredicateCache::new();
        let preds = make_preds(Rgb([255, 0, 0]));
        let assets = BTreeMap::new();
        let frame = red_pixel_frame();
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "red_here".into(),
            ms: 500,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 1000,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = ElapsedMsSinceTrueEvaluator.evaluate(&p, &ctx);
        assert!(matches!(r, Err(PerceptionError::AssetDecode(_))));
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p vcli-perception --lib elapsed`
Expected: 4 tests passing.

- [ ] **Step 6: Commit**

```bash
git add crates/vcli-perception/src/elapsed.rs crates/vcli-perception/src/evaluator.rs \
        crates/vcli-perception/src/color_at.rs crates/vcli-perception/src/pixel_diff.rs \
        crates/vcli-perception/src/logical.rs crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: ElapsedMsSinceTrueEvaluator + program-scoped EvalCtx"
```

---

### Task 12: `TemplateEvaluator` (Tier 2, imageproc NCC)

**Files:**
- Create: `crates/vcli-perception/src/template.rs`
- Create: `crates/vcli-perception/tests/fixtures/README.md`
- Create: `crates/vcli-perception/tests/fixtures/skip_button_40x16.png` (generated programmatically in tests — see Step 3)
- Modify: `crates/vcli-perception/src/lib.rs`

- [ ] **Step 1: Add module + re-export**

Append to `lib.rs`:

```rust
pub mod template;

pub use template::TemplateEvaluator;
```

- [ ] **Step 2: Write `template.rs`**

```rust
//! `TemplateEvaluator` — Tier 2. NCC template matching via
//! `imageproc::template_matching::match_template` with
//! `MatchTemplateMethod::CrossCorrelationNormalized`.
//!
//! Spec §"Template matching": `imageproc` NCC, region-scoped by default
//! (the runtime passes the region as `Region::Absolute` after resolving
//! `window` / `relative_to`). Confidence is an inclusive threshold in
//! [0, 1]. v0 **does not** implement pyramid search — Decision 4.2
//! defers that to the runtime/submit pipeline (it only affects how
//! templates are stored, not the evaluator's interface).

use image::{imageops, GrayImage};
use imageproc::template_matching::{
    find_extremes, match_template, MatchTemplateMethod,
};

use vcli_core::geom::Rect;
use vcli_core::predicate::{Confidence, PredicateKind};
use vcli_core::{MatchData, Predicate, PredicateResult, Region};

use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::frame_view::{crop_rgb, frame_to_rgb};

/// Evaluator for `PredicateKind::Template`.
#[derive(Debug, Default)]
pub struct TemplateEvaluator;

impl TemplateEvaluator {
    /// Decode a PNG byte-slice into a grayscale image. Used by the
    /// evaluator to load the asset the daemon materialized into
    /// `EvalCtx::assets`.
    ///
    /// # Errors
    ///
    /// Returns `AssetDecode` if `image::load_from_memory` fails.
    pub fn decode_gray(bytes: &[u8]) -> Result<GrayImage> {
        let dyn_img = image::load_from_memory(bytes)
            .map_err(|e| PerceptionError::AssetDecode(e.to_string()))?;
        Ok(dyn_img.to_luma8())
    }
}

impl Evaluator for TemplateEvaluator {
    fn evaluate(&self, predicate: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
        let PredicateKind::Template {
            image: image_ref,
            confidence,
            region,
            throttle_ms: _,
        } = predicate
        else {
            return Err(PerceptionError::AssetDecode(
                "template evaluator got wrong kind".into(),
            ));
        };

        // Resolve the asset bytes. Daemon strips the "sha256:" prefix
        // before handing bytes to us, so the key is the hex digest.
        let key = image_ref.strip_prefix("sha256:").unwrap_or(image_ref);
        let bytes = ctx
            .assets
            .get(key)
            .ok_or_else(|| PerceptionError::AssetNotMaterialized(image_ref.clone()))?;
        let template_gray = Self::decode_gray(bytes)?;

        // Select the haystack: either the full frame or a cropped region.
        let (haystack_rgb, region_origin) = match region {
            Region::Absolute { rect } => {
                let crop = crop_rgb(ctx.frame, *rect)?;
                (
                    crop,
                    (
                        rect.x.max(ctx.frame.bounds.x),
                        rect.y.max(ctx.frame.bounds.y),
                    ),
                )
            }
            // Window / RelativeTo should have been resolved to Absolute
            // by the runtime before this evaluator is called. Treat as
            // the whole frame as a graceful fallback.
            _ => (
                frame_to_rgb(ctx.frame)?,
                (ctx.frame.bounds.x, ctx.frame.bounds.y),
            ),
        };
        let haystack_gray = imageops::grayscale(&haystack_rgb);

        // Haystack must be at least as big as the template.
        if haystack_gray.width() < template_gray.width()
            || haystack_gray.height() < template_gray.height()
        {
            return Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            });
        }

        // Normalized cross-correlation. Returns a response map where
        // higher values = better match; max is 1.0 for a perfect match.
        let result_map = match_template(
            &haystack_gray,
            &template_gray,
            MatchTemplateMethod::CrossCorrelationNormalized,
        );
        let extremes = find_extremes(&result_map);
        let (best_val, best_loc) = (extremes.max_value, extremes.max_value_location);

        // `best_val` is in [0, 1] for normalized cross-correlation.
        let best_val_f64 = f64::from(best_val);
        let threshold = confidence.0;

        if best_val_f64 < threshold {
            return Ok(PredicateResult {
                truthy: false,
                match_data: None,
                at: ctx.now_ms,
            });
        }

        // Translate the match location from haystack-local to absolute
        // screen coordinates.
        let bbox = Rect {
            x: region_origin.0 + best_loc.0 as i32,
            y: region_origin.1 + best_loc.1 as i32,
            w: template_gray.width() as i32,
            h: template_gray.height() as i32,
        };
        Ok(PredicateResult {
            truthy: true,
            match_data: Some(MatchData {
                bbox,
                confidence: Confidence(best_val_f64),
            }),
            at: ctx.now_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use image::{ImageBuffer, Luma, Rgb, RgbImage};

    use vcli_core::geom::Rect;
    use vcli_core::{Frame, FrameFormat, Predicate, Region};

    use crate::cache::PredicateCache;
    use crate::state::PerceptionState;

    /// Build a 64×64 frame containing a 16×8 black rectangle at (20, 20)
    /// on a white background, encoded as RGBA8.
    fn scene_with_box() -> Frame {
        let w = 64usize;
        let h = 64usize;
        let stride = w * 4;
        let mut pixels = vec![255u8; stride * h];
        for y in 20..28 {
            for x in 20..36 {
                let off = y * stride + x * 4;
                pixels[off] = 0; // R
                pixels[off + 1] = 0;
                pixels[off + 2] = 0;
                pixels[off + 3] = 255;
            }
        }
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: w as i32,
                h: h as i32,
            },
            stride,
            Arc::from(pixels),
            0,
        )
    }

    /// Build a PNG byte-stream for a 16×8 all-black template.
    fn template_png_black_16x8() -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(16, 8, Rgb([0, 0, 0]));
        let mut bytes: Vec<u8> = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        bytes
    }

    fn template_png_white_16x8() -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(16, 8, Rgb([255, 255, 255]));
        let mut bytes: Vec<u8> = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        bytes
    }

    #[test]
    fn decode_gray_parses_valid_png() {
        let png = template_png_black_16x8();
        let g = TemplateEvaluator::decode_gray(&png).unwrap();
        assert_eq!(g.width(), 16);
        assert_eq!(g.height(), 8);
    }

    #[test]
    fn decode_gray_errors_on_garbage() {
        let r = TemplateEvaluator::decode_gray(b"not a png");
        assert!(matches!(r, Err(PerceptionError::AssetDecode(_))));
    }

    #[test]
    fn template_match_found_in_scene_reports_correct_bbox() {
        let frame = scene_with_box();
        let template_bytes = template_png_black_16x8();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let mut assets = BTreeMap::new();
        assets.insert("blackbox".into(), template_bytes);
        let p = PredicateKind::Template {
            image: "sha256:blackbox".into(),
            confidence: Confidence(0.9),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
            },
            throttle_ms: 200,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 42,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = TemplateEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(r.truthy);
        let m = r.match_data.expect("match_data populated on truthy");
        assert_eq!(m.bbox.w, 16);
        assert_eq!(m.bbox.h, 8);
        assert_eq!(m.bbox.x, 20);
        assert_eq!(m.bbox.y, 20);
        assert!(m.confidence.0 >= 0.9);
    }

    #[test]
    fn template_missing_in_scene_is_falsy() {
        let frame = scene_with_box(); // black-box-on-white
        let template_bytes = template_png_white_16x8(); // all-white template never correlates high
        // Actually: NCC of all-white against a region with all-white pixels
        // can return 1.0 trivially, BUT find_extremes on constant images
        // returns NaN due to zero variance — imageproc guards this by
        // returning 0. Prefer using confidence 0.99 + a scene whose
        // whitespace regions still won't exceed it. Simpler: use a
        // distinctive template instead. Use a 32×32 gradient template
        // that cannot be found in a 64x64 solid scene.
        let _ = template_bytes; // Unused: we build a different template below.

        use image::Rgb;
        let mut grad: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(32, 32);
        for y in 0..32u32 {
            for x in 0..32u32 {
                grad.put_pixel(x, y, Rgb([((x * 8) % 255) as u8, ((y * 8) % 255) as u8, 128]));
            }
        }
        let mut grad_bytes: Vec<u8> = Vec::new();
        grad.write_to(
            &mut std::io::Cursor::new(&mut grad_bytes),
            image::ImageFormat::Png,
        )
        .unwrap();

        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let mut assets = BTreeMap::new();
        assets.insert("grad".into(), grad_bytes);
        let p = PredicateKind::Template {
            image: "sha256:grad".into(),
            confidence: Confidence(0.99),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
            },
            throttle_ms: 200,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 42,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = TemplateEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy);
        assert!(r.match_data.is_none());
    }

    #[test]
    fn template_larger_than_haystack_is_falsy() {
        let frame = scene_with_box();
        // Build a template larger than the search region.
        let big: ImageBuffer<Luma<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(200, 200, Luma([0]));
        let mut bytes: Vec<u8> = Vec::new();
        big.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let mut assets = BTreeMap::new();
        assets.insert("big".into(), bytes);
        let p = PredicateKind::Template {
            image: "sha256:big".into(),
            confidence: Confidence(0.5),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
            },
            throttle_ms: 200,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 42,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = TemplateEvaluator.evaluate(&p, &ctx).unwrap();
        assert!(!r.truthy);
    }

    #[test]
    fn missing_asset_errors() {
        let frame = scene_with_box();
        let cache = PredicateCache::new();
        let state = PerceptionState::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new(); // no "blackbox" entry
        let p = PredicateKind::Template {
            image: "sha256:blackbox".into(),
            confidence: Confidence(0.9),
            region: Region::Absolute {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
            },
            throttle_ms: 200,
        };
        let ctx = EvalCtx {
            frame: &frame,
            now_ms: 42,
            cache: &cache,
            state: &state,
            predicates: &preds,
            assets: &assets,
            program: None,
        };
        let r = TemplateEvaluator.evaluate(&p, &ctx);
        assert!(matches!(r, Err(PerceptionError::AssetNotMaterialized(_))));
    }
}
```

- [ ] **Step 3: Write fixtures README (templates generated in-test, no binary assets checked in)**

Create `crates/vcli-perception/tests/fixtures/README.md`:

```markdown
# Test fixtures for vcli-perception

All PNG fixtures used by tests in this crate are generated **programmatically**
in the test code — we deliberately do NOT check in binary assets for unit
tests. See:

- `src/template.rs` — `template_png_black_16x8()` and `template_png_white_16x8()`
  build tiny PNGs at runtime via the `image` crate.
- `src/pixel_diff.rs` — `solid_frame()` builds in-memory RGBA frames.

Integration fixtures (the YT skip-button PNG used in e2e demos) live under
`/assets/fixtures/` at the workspace root, not here.
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p vcli-perception --lib template`
Expected: 5 tests passing.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-perception/src/template.rs \
        crates/vcli-perception/tests/fixtures/README.md \
        crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: TemplateEvaluator (imageproc NCC + bbox recovery)"
```

---

### Task 13: `Perception` façade + `evaluate_named` + `dispatch_leaf`

**Files:**
- Modify: `crates/vcli-perception/src/perception.rs`

- [ ] **Step 1: Replace the stub with the real façade**

Replace the contents of `crates/vcli-perception/src/perception.rs`:

```rust
//! Public façade. The runtime asks for a named predicate's result; the
//! façade canonicalizes, checks the cache, dispatches to the right
//! evaluator, memoizes the result, and returns it. Cache is wiped at
//! tick boundaries by the runtime calling `PredicateCache::clear()`.

use std::collections::BTreeMap;
use std::sync::Arc;

use vcli_core::clock::UnixMs;
use vcli_core::predicate::PredicateKind;
use vcli_core::{
    predicate_hash, Frame, Predicate, PredicateHash, PredicateResult, ProgramId,
};

use crate::cache::PredicateCache;
use crate::color_at::ColorAtEvaluator;
use crate::elapsed::ElapsedMsSinceTrueEvaluator;
use crate::error::{PerceptionError, Result};
use crate::evaluator::{EvalCtx, Evaluator};
use crate::logical::{AllOfEvaluator, AnyOfEvaluator, NotEvaluator};
use crate::pixel_diff::PixelDiffEvaluator;
use crate::state::PerceptionState;
use crate::template::TemplateEvaluator;

/// Top-level perception entry point. Holds the per-tick cache and a handle
/// to cross-tick state.
pub struct Perception {
    cache: Arc<PredicateCache>,
    state: Arc<PerceptionState>,
}

impl Perception {
    /// Create a fresh perception with empty cache + state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: Arc::new(PredicateCache::new()),
            state: Arc::new(PerceptionState::new()),
        }
    }

    /// Reuse existing shared state (e.g. after a runtime restart that
    /// wants to preserve first-true timers across a short gap).
    #[must_use]
    pub fn with_state(state: Arc<PerceptionState>) -> Self {
        Self {
            cache: Arc::new(PredicateCache::new()),
            state,
        }
    }

    /// Borrow the cache. The runtime calls `.clear()` at tick start.
    #[must_use]
    pub fn cache(&self) -> &PredicateCache {
        &self.cache
    }

    /// Borrow the cross-tick state. Diagnostic use only.
    #[must_use]
    pub fn state(&self) -> &PerceptionState {
        &self.state
    }

    /// Wipe the per-tick cache. Called by the runtime at the start of
    /// every tick. Cross-tick state is preserved.
    pub fn clear(&self) {
        self.cache.clear();
    }

    /// Evaluate a named predicate, memoizing through the shared cache.
    ///
    /// # Errors
    ///
    /// Propagates evaluator errors — see `PerceptionError`.
    pub fn evaluate_named(
        &self,
        name: &str,
        predicates: &BTreeMap<String, Predicate>,
        frame: &Frame,
        now_ms: UnixMs,
        assets: &BTreeMap<String, Vec<u8>>,
        program: Option<ProgramId>,
    ) -> Result<PredicateResult> {
        let pred = predicates
            .get(name)
            .ok_or_else(|| PerceptionError::UnknownPredicate(name.into()))?;
        let ctx = EvalCtx {
            frame,
            now_ms,
            cache: &self.cache,
            state: &self.state,
            predicates,
            assets,
            program,
        };
        let hash = hash_predicate(pred)?;
        if let Some(cached) = self.cache.get(&hash) {
            return Ok(cached);
        }
        let result = dispatch(pred, &ctx)?;
        self.cache.insert(hash, result.clone());
        Ok(result)
    }
}

impl Default for Perception {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash any predicate via the canonical JSON pipeline in `vcli-core`.
fn hash_predicate(p: &Predicate) -> Result<PredicateHash> {
    let v = serde_json::to_value(p).map_err(|e| {
        PerceptionError::AssetDecode(format!("serialize predicate for hash: {e}"))
    })?;
    predicate_hash(&v)
        .map_err(|e| PerceptionError::AssetDecode(format!("hash predicate: {e}")))
}

/// Dispatch a predicate to its evaluator. Internal — logical evaluators
/// call `dispatch_leaf` for non-logical kinds while they handle their
/// own recursion.
pub(crate) fn dispatch(p: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
    match p {
        PredicateKind::ColorAt { .. } => ColorAtEvaluator.evaluate(p, ctx),
        PredicateKind::PixelDiff { .. } => PixelDiffEvaluator.evaluate(p, ctx),
        PredicateKind::Template { .. } => TemplateEvaluator.evaluate(p, ctx),
        PredicateKind::AllOf { .. } => AllOfEvaluator.evaluate(p, ctx),
        PredicateKind::AnyOf { .. } => AnyOfEvaluator.evaluate(p, ctx),
        PredicateKind::Not { .. } => NotEvaluator.evaluate(p, ctx),
        PredicateKind::ElapsedMsSinceTrue { .. } => ElapsedMsSinceTrueEvaluator.evaluate(p, ctx),
    }
}

/// Non-logical dispatch. Called by `logical.rs` during recursion.
pub(crate) fn dispatch_leaf(p: &Predicate, ctx: &EvalCtx<'_>) -> Result<PredicateResult> {
    match p {
        PredicateKind::ColorAt { .. } => ColorAtEvaluator.evaluate(p, ctx),
        PredicateKind::PixelDiff { .. } => PixelDiffEvaluator.evaluate(p, ctx),
        PredicateKind::Template { .. } => TemplateEvaluator.evaluate(p, ctx),
        PredicateKind::ElapsedMsSinceTrue { .. } => ElapsedMsSinceTrueEvaluator.evaluate(p, ctx),
        PredicateKind::AllOf { .. } | PredicateKind::AnyOf { .. } | PredicateKind::Not { .. } => {
            // Logical kinds shouldn't flow through dispatch_leaf — the
            // logical evaluators handle their own sub-children. Defensive
            // fallback so we don't silently return the wrong thing.
            dispatch(p, ctx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use vcli_core::geom::{Point, Rect};
    use vcli_core::predicate::Rgb;
    use vcli_core::{Frame, FrameFormat};

    fn red_frame() -> Frame {
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            4,
            Arc::from(vec![255u8, 0, 0, 255]),
            0,
        )
    }

    #[test]
    fn evaluate_named_returns_truthy_for_matching_color() {
        let p = Perception::new();
        let mut preds = BTreeMap::new();
        preds.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let frame = red_frame();
        let r = p
            .evaluate_named("red_here", &preds, &frame, 123, &assets, None)
            .unwrap();
        assert!(r.truthy);
        assert_eq!(r.at, 123);
    }

    #[test]
    fn evaluate_named_unknown_errors() {
        let p = Perception::new();
        let preds = BTreeMap::new();
        let assets = BTreeMap::new();
        let frame = red_frame();
        let r = p.evaluate_named("nope", &preds, &frame, 0, &assets, None);
        assert!(matches!(r, Err(PerceptionError::UnknownPredicate(_))));
    }

    #[test]
    fn second_call_for_same_name_is_a_cache_hit() {
        let p = Perception::new();
        let mut preds = BTreeMap::new();
        preds.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let frame = red_frame();
        let r1 = p
            .evaluate_named("red_here", &preds, &frame, 100, &assets, None)
            .unwrap();
        let r2 = p
            .evaluate_named("red_here", &preds, &frame, 200, &assets, None)
            .unwrap();
        // Same hash → same cached result → `at` timestamp preserved from
        // first eval, not updated to 200. This proves the cache hit.
        assert_eq!(r1.at, 100);
        assert_eq!(r2.at, 100);
    }

    #[test]
    fn clear_wipes_cache_but_not_state() {
        let p = Perception::new();
        let mut preds = BTreeMap::new();
        preds.insert(
            "red_here".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let frame = red_frame();
        let _ = p
            .evaluate_named("red_here", &preds, &frame, 100, &assets, None)
            .unwrap();
        assert_eq!(p.cache().len(), 1);
        p.clear();
        assert_eq!(p.cache().len(), 0);
        // State is preserved — recording a snapshot then clearing cache
        // keeps the snapshot accessible.
        let h = super::hash_predicate(
            &PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        )
        .unwrap();
        p.state().record_snapshot(
            h.clone(),
            crate::state::PriorSnapshot {
                dhash: 42,
                at_ms: 1000,
            },
        );
        p.clear();
        assert!(p.state().prior_snapshot(&h).is_some());
    }

    #[test]
    fn two_programs_with_same_predicate_share_cache() {
        // If two programs both reference a predicate with the same canonical
        // JSON form (same kind, same params), the second evaluation should
        // hit the cache.
        let p = Perception::new();
        let mut preds = BTreeMap::new();
        preds.insert(
            "a".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        preds.insert(
            "b_also_red".into(),
            PredicateKind::ColorAt {
                point: Point { x: 0, y: 0 },
                rgb: Rgb([255, 0, 0]),
                tolerance: 0,
            },
        );
        let assets = BTreeMap::new();
        let frame = red_frame();
        let _ = p
            .evaluate_named("a", &preds, &frame, 100, &assets, None)
            .unwrap();
        let _ = p
            .evaluate_named("b_also_red", &preds, &frame, 200, &assets, None)
            .unwrap();
        // Only one entry — both names hashed to the same PredicateHash.
        assert_eq!(p.cache().len(), 1);
    }
}
```

- [ ] **Step 2: Add `Perception` re-export**

Append to `lib.rs`:

```rust
pub use perception::Perception;
```

- [ ] **Step 3: Run façade tests**

Run: `cargo test -p vcli-perception --lib perception`
Expected: 5 tests passing.

- [ ] **Step 4: Run the whole crate test suite**

Run: `cargo test -p vcli-perception`
Expected: all tests (~35–40) passing.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-perception/src/perception.rs crates/vcli-perception/src/lib.rs
git commit -m "vcli-perception: Perception façade — evaluate_named with cache memoization"
```

---

### Task 14: Integration test — multi-program cache dedup + per-tick clear

**Files:**
- Create: `crates/vcli-perception/tests/cache_and_state.rs`

- [ ] **Step 1: Write the integration test**

Create `crates/vcli-perception/tests/cache_and_state.rs`:

```rust
//! Integration tests that exercise Perception as a whole:
//!
//! - Multi-program cache dedup for identical predicate canonical forms
//! - Per-tick cache wipe via `clear()`
//! - Cross-tick PerceptionState preservation for elapsed_ms_since_true

use std::collections::BTreeMap;
use std::sync::Arc;

use vcli_core::geom::{Point, Rect};
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::{Frame, FrameFormat, Predicate, ProgramId};

use vcli_perception::Perception;

fn red_frame() -> Frame {
    Frame::new(
        FrameFormat::Rgba8,
        Rect {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        },
        4,
        Arc::from(vec![255u8, 0, 0, 255]),
        0,
    )
}

fn color_pred(r: u8, g: u8, b: u8) -> Predicate {
    PredicateKind::ColorAt {
        point: Point { x: 0, y: 0 },
        rgb: Rgb([r, g, b]),
        tolerance: 0,
    }
}

#[test]
fn cache_dedups_across_distinct_program_predicate_names() {
    let p = Perception::new();
    let mut preds = BTreeMap::new();
    // Two programs, each with their own local name, but identical canonical form.
    preds.insert("prog_a_red".into(), color_pred(255, 0, 0));
    preds.insert("prog_b_red".into(), color_pred(255, 0, 0));
    let assets = BTreeMap::new();
    let frame = red_frame();
    let _ = p
        .evaluate_named("prog_a_red", &preds, &frame, 100, &assets, None)
        .unwrap();
    let _ = p
        .evaluate_named("prog_b_red", &preds, &frame, 200, &assets, None)
        .unwrap();
    assert_eq!(p.cache().len(), 1, "identical canonical predicates dedupe");
}

#[test]
fn per_tick_clear_invalidates_results() {
    let p = Perception::new();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), color_pred(255, 0, 0));
    let assets = BTreeMap::new();
    let frame = red_frame();
    let r1 = p
        .evaluate_named("red", &preds, &frame, 100, &assets, None)
        .unwrap();
    p.clear();
    let r2 = p
        .evaluate_named("red", &preds, &frame, 200, &assets, None)
        .unwrap();
    // Without cache clear, r2.at would equal r1.at (100). With it, r2 is
    // freshly evaluated at 200.
    assert_eq!(r1.at, 100);
    assert_eq!(r2.at, 200);
}

#[test]
fn elapsed_ms_since_true_persists_across_tick_clears() {
    let p = Perception::new();
    let pid = ProgramId::new();
    let mut preds = BTreeMap::new();
    preds.insert("red".into(), color_pred(255, 0, 0));
    preds.insert(
        "stable_500".into(),
        PredicateKind::ElapsedMsSinceTrue {
            predicate: "red".into(),
            ms: 500,
        },
    );
    let assets = BTreeMap::new();
    let frame = red_frame();

    // Tick 1 at t=1000: child true, 0ms elapsed → stable_500 is false.
    let r = p
        .evaluate_named("stable_500", &preds, &frame, 1000, &assets, Some(pid))
        .unwrap();
    assert!(!r.truthy);

    // End of tick — runtime clears the cache.
    p.clear();

    // Tick 2 at t=1499: 499ms elapsed, still false.
    let r = p
        .evaluate_named("stable_500", &preds, &frame, 1499, &assets, Some(pid))
        .unwrap();
    assert!(!r.truthy);

    p.clear();

    // Tick 3 at t=1500: exactly 500ms — truthy.
    let r = p
        .evaluate_named("stable_500", &preds, &frame, 1500, &assets, Some(pid))
        .unwrap();
    assert!(r.truthy);
}
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test -p vcli-perception --test cache_and_state`
Expected: 3 tests passing.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-perception/tests/cache_and_state.rs
git commit -m "vcli-perception: integration tests for cache dedup + per-tick clear + state persistence"
```

---

### Task 15: Full-crate verification

**Files:** (none new — verification)

- [ ] **Step 1: Run every perception test**

Run: `cargo test -p vcli-perception`
Expected: ~40 tests across all modules, all passing.

- [ ] **Step 2: Run clippy in pedantic mode**

Run: `cargo clippy -p vcli-perception --all-targets -- -D warnings`
Expected: no warnings, no errors. Common places to tighten: `as` casts in `pixel_diff.rs` and `frame_view.rs` — use `u32::try_from` / `i32::try_from` where clippy flags them.

- [ ] **Step 3: Run rustfmt check**

Run: `cargo fmt --all -- --check`
Expected: no diff. If any file differs, run `cargo fmt --all` and commit separately.

- [ ] **Step 4: Verify docs build clean**

Run: `cargo doc -p vcli-perception --no-deps`
Expected: builds without warnings (the `#![warn(missing_docs)]` in `lib.rs` enforces this).

- [ ] **Step 5: Verify workspace check still passes**

Run: `cargo check --workspace`
Expected: clean — `vcli-core` and `vcli-perception` both compile.

- [ ] **Step 6: Verify CI-equivalent run locally**

Run in order:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

Expected: all three exit 0.

- [ ] **Step 7: Tag the milestone**

```bash
git tag lane-g-vcli-perception-complete -m "vcli-perception complete — Tier-1 + Tier-2 evaluators wired"
```

---

## What this plan unlocks

Once this plan completes, `vcli-runtime` can:

- Own a single `Perception` instance per tick loop thread.
- Call `perception.clear()` at the start of every tick.
- Iterate over active predicates and, via `evaluate_named`, get results that dedup across programs by canonical JSON.
- Bridge its `Clock` to `now_ms` at tick start.
- Pass the decoded asset map (built by `vcli-store` after asset ingestion) as the `assets` argument.
- Resolve `Region::Window` / `Region::RelativeTo` to `Region::Absolute` before calling Perception — this crate only handles `Absolute`.

Post-v0 (v0.2+), OCR and VLM evaluators slot in by implementing `Evaluator` and being wired into `perception::dispatch` as new match arms — no trait changes required.

---

## Self-review checklist

- Every task shows the actual code; no TODO / TBD / "fill in later".
- Evaluator trait takes `&self` (Decision A) — enables `rayon::par_iter` in the runtime without any changes here.
- `PredicateCache` is `DashMap<PredicateHash, PredicateResult>` (Decision A).
- `PerceptionState` holds cross-tick state (first-true timers, prior snapshots); `PredicateCache` is per-tick with a `clear()` called by the runtime.
- Tier-1 coverage: `ColorAtEvaluator`, `PixelDiffEvaluator`, `AllOf/AnyOf/Not`, `ElapsedMsSinceTrue`.
- Tier-2 coverage: `TemplateEvaluator` via `imageproc::template_matching::match_template` with `MatchTemplateMethod::CrossCorrelationNormalized`, max located via `find_extremes`.
- Template bytes arrive via `EvalCtx::assets` (keyed by hex digest, not `sha256:` prefix) — this crate has no filesystem access (Decision F4: DSL/perception stay pure, daemon submit orchestrates IO).
- Shared-cache recursion through logical evaluators: `logical.rs::evaluate_named_with_depth` checks the cache before re-evaluating children, so two `all_of` siblings sharing a child pay only once.
- `elapsed_ms_since_true` state is program-local (keyed by `(ProgramId, predicate_name)`) per spec §"`elapsed_ms_since_true`" — the façade threads `program: Option<ProgramId>` into `EvalCtx`, and evaluators error if `None` when needed.
- All tests compile and pass: each task runs `cargo test -p vcli-perception …` before commit.
- Template fixtures are generated programmatically in-test (no binary assets checked in) — keeps the fixture README honest and the repo light.
- CI-equivalent locally verified in Task 15.
- Zero edits to `vcli-core`. Plan is implementable on a branch off master in parallel with lanes B / C / D / E / F.
- All Decision references (A, 1.1, 1.3, 1.6, 4.2, F4) point to the authoritative spec appendix at `docs/superpowers/specs/2026-04-16-vcli-design.md` §"Review decisions — 2026-04-16".
