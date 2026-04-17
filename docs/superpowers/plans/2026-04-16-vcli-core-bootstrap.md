# vcli Phase 0 + Phase 1: Workspace Bootstrap + `vcli-core` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Cargo workspace, CI, and the `vcli-core` crate — shared types, canonical JSON, predicate hashing, clock abstraction, event and error taxonomies — fully unit-tested so every downstream crate can depend on it in parallel.

**Architecture:** `vcli-core` is a zero-runtime-dep types crate. Every public type has `serde` derives and round-trip tests. Canonical JSON produces stable, reproducible bytes for `PredicateHash` (Decision 1.1). A `Clock` trait lives here (Decision 1.6) with a real and a test impl. The crate compiles on stable Rust, passes `clippy -D warnings` and `rustfmt --check`, and is the only thing downstream crates need available to start work in parallel worktrees.

**Tech Stack:** Rust (stable, 2021 edition), Cargo workspace, `serde`, `serde_json`, `thiserror`, `uuid` (v4), `ryu` (canonical number formatting), `unicode-normalization` (NFC for canonical strings), dev-only `proptest`. No `tokio`, no `rayon`, no `dashmap` in this crate — those land in downstream crates.

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md`. When this plan references a decision by number (e.g. "Decision 1.1"), see the "Review decisions — 2026-04-16" appendix in that file.

---

## File structure produced by this plan

```
vcli/
├── .github/workflows/ci.yml
├── .gitignore
├── Cargo.toml                       # workspace
├── LICENSE                          # MIT
├── README.md                        # minimal stub
├── rust-toolchain.toml
├── rustfmt.toml
├── crates/
│   └── vcli-core/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs               # module tree + re-exports
│           ├── ids.rs               # ProgramId, ProgramIdParseError
│           ├── geom.rs              # Point, Rect
│           ├── frame.rs             # Frame (capture output)
│           ├── clock.rs             # Clock, SystemClock, TestClock
│           ├── region.rs            # Region (absolute | window | relative_to)
│           ├── predicate.rs         # Predicate, PredicateKind, PredicateResult, MatchData
│           ├── action.rs            # InputAction, Button, Modifier
│           ├── step.rs              # Step (click | type | key | scroll | move | wait_for | assert | sleep_ms)
│           ├── watch.rs             # Watch, Lifetime
│           ├── trigger.rs           # Trigger (on_submit | on_predicate | manual)
│           ├── state.rs             # ProgramState
│           ├── program.rs           # Program (top-level DSL document)
│           ├── canonical.rs         # canonical JSON, PredicateHash
│           ├── events.rs            # Event enum (daemon-wide taxonomy)
│           └── error.rs             # Error, ErrorCode
└── fixtures/
    └── yt_ad_skipper.json           # copied from spec for round-trip tests
```

**Responsibility split rationale:** each file is one topic, small enough to hold in context, and testable in isolation. No file should exceed ~300 lines in this crate. Keep `lib.rs` to re-exports and a brief module doc — no logic.

---

## Phase 0 — Bootstrap

### Task 0.1: Initialize workspace skeleton

**Files:**
- Create: `.gitignore`
- Create: `Cargo.toml` (workspace root)
- Create: `rust-toolchain.toml`
- Create: `rustfmt.toml`
- Create: `README.md`
- Create: `LICENSE`

- [ ] **Step 1: Verify clean state**

Run: `git status` and `ls`
Expected: git already initialized (repo has existing commits); only `docs/` and `TODOS.md` present.

- [ ] **Step 2: Write `.gitignore`**

Create `.gitignore` with:

```
/target
Cargo.lock.orig
**/*.rs.bk
.DS_Store
*.swp
# IDE
.idea/
.vscode/
# Daemon test data
/tmp-data/
/fuzz/target/
/fuzz/corpus/
/fuzz/artifacts/
```

- [ ] **Step 3: Write `rust-toolchain.toml`**

Create `rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

- [ ] **Step 4: Write `rustfmt.toml`**

Create `rustfmt.toml`:

```toml
edition = "2021"
max_width = 100
use_field_init_shorthand = true
use_try_shorthand = true
```

- [ ] **Step 5: Write `Cargo.toml` workspace manifest**

Create `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/vcli-core",
]

[workspace.package]
edition = "2021"
rust-version = "1.75"
version = "0.0.1"
license = "MIT"
repository = "https://github.com/blaksmatic/vcli"
authors = ["blaksmatic"]

[workspace.dependencies]
# Shared across all crates that will exist. Only vcli-core uses these in this plan.
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["preserve_order"] }
thiserror = "2"
uuid = { version = "1", features = ["v4", "serde"] }
ryu = "1"
unicode-normalization = "0.1"

# Dev-only
proptest = "1"
```

- [ ] **Step 6: Write `README.md`**

Create `README.md`:

```markdown
# vcli — Vision CLI

A local, persistent screen-control runtime that agents command through declarative JSON programs.

**Status:** pre-alpha (v0.1 in progress).

See [`docs/superpowers/specs/2026-04-16-vcli-design.md`](docs/superpowers/specs/2026-04-16-vcli-design.md) for the design.
```

- [ ] **Step 7: Write `LICENSE` (MIT)**

Create `LICENSE` with a standard MIT license. Fill in `2026 blaksmatic` as copyright holder. (Use any canonical MIT text, e.g. the one at https://opensource.org/licenses/MIT — copy verbatim with copyright line updated.)

- [ ] **Step 8: Verify workspace parses**

Run: `cargo check --workspace`
Expected: fails — `workspace members must exist`. Accept; we create the member next.

- [ ] **Step 9: Commit bootstrap**

```bash
git add .gitignore Cargo.toml rust-toolchain.toml rustfmt.toml README.md LICENSE
git commit -m "Bootstrap Cargo workspace + toolchain + license"
```

---

### Task 0.2: Create empty `vcli-core` crate

**Files:**
- Create: `crates/vcli-core/Cargo.toml`
- Create: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Write `crates/vcli-core/Cargo.toml`**

Create `crates/vcli-core/Cargo.toml`:

```toml
[package]
name = "vcli-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Shared types, canonical JSON, clock, and event/error taxonomy for vcli."

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true }
ryu = { workspace = true }
unicode-normalization = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
```

- [ ] **Step 2: Write minimal `crates/vcli-core/src/lib.rs`**

```rust
//! vcli-core — shared types, canonical JSON, clock abstraction, event taxonomy.
//!
//! See the v0 design spec at `docs/superpowers/specs/2026-04-16-vcli-design.md`
//! for the authoritative definitions implemented here.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p vcli-core`
Expected: OK, no errors (may warn about empty lib).

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/Cargo.toml crates/vcli-core/src/lib.rs
git commit -m "vcli-core: empty crate shell"
```

---

### Task 0.3: GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write CI workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [master, main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: -D warnings

jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all -- --check

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --all-targets -- -D warnings

  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --locked
```

*(Decision 3.1 adds `cargo fuzz run dsl_validator` on PRs and the release workflow from Decision 0A — both land in downstream plans when `vcli-dsl` and the release pipeline exist. This plan only introduces the foundation.)*

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "CI: add fmt / clippy / test on ubuntu + macOS"
```

---

## Phase 1 — `vcli-core` implementation

Every task in Phase 1 follows strict TDD: write the failing test first, run it to confirm failure, implement the minimum to pass, re-run, then commit.

### Task 1.1: Point + Rect

**Files:**
- Create: `crates/vcli-core/src/geom.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `mod geom;` to `lib.rs`**

Append to `crates/vcli-core/src/lib.rs`:

```rust
pub mod geom;
```

- [ ] **Step 2: Write failing tests in `crates/vcli-core/src/geom.rs`**

```rust
//! Geometric primitives: integer Point and Rect.
//!
//! Coordinates are in logical (1x) pixels. Capture converts physical→logical
//! at the capture boundary per Decision F1/4.3; everything above `vcli-capture`
//! operates in logical space.

use serde::{Deserialize, Serialize};

/// A point in logical (1x) pixels. Top-left origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate (pixels, left→right).
    pub x: i32,
    /// Vertical coordinate (pixels, top→bottom).
    pub y: i32,
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge (pixels).
    pub x: i32,
    /// Top edge (pixels).
    pub y: i32,
    /// Width (pixels, non-negative in valid rects).
    pub w: i32,
    /// Height (pixels, non-negative in valid rects).
    pub h: i32,
}

impl Rect {
    /// Center point of the rectangle (integer-rounded toward zero).
    #[must_use]
    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2,
            y: self.y + self.h / 2,
        }
    }

    /// Top-left corner.
    #[must_use]
    pub fn top_left(&self) -> Point {
        Point { x: self.x, y: self.y }
    }

    /// Whether this rect contains the given point (inclusive of top/left, exclusive of bottom/right).
    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_center_is_correct_for_even_dims() {
        let r = Rect { x: 0, y: 0, w: 100, h: 40 };
        assert_eq!(r.center(), Point { x: 50, y: 20 });
    }

    #[test]
    fn rect_center_rounds_toward_zero_for_odd_dims() {
        let r = Rect { x: 10, y: 20, w: 5, h: 7 };
        assert_eq!(r.center(), Point { x: 12, y: 23 });
    }

    #[test]
    fn rect_top_left_reports_origin() {
        let r = Rect { x: 3, y: 4, w: 10, h: 10 };
        assert_eq!(r.top_left(), Point { x: 3, y: 4 });
    }

    #[test]
    fn rect_contains_is_inclusive_top_left_exclusive_bottom_right() {
        let r = Rect { x: 0, y: 0, w: 10, h: 10 };
        assert!(r.contains(Point { x: 0, y: 0 }));
        assert!(r.contains(Point { x: 9, y: 9 }));
        assert!(!r.contains(Point { x: 10, y: 5 }));
        assert!(!r.contains(Point { x: 5, y: 10 }));
    }

    #[test]
    fn point_serde_roundtrip() {
        let p = Point { x: -5, y: 7 };
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(j, r#"{"x":-5,"y":7}"#);
        let back: Point = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn rect_serde_roundtrip() {
        let r = Rect { x: 1, y: 2, w: 3, h: 4 };
        let j = serde_json::to_string(&r).unwrap();
        let back: Rect = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }
}
```

- [ ] **Step 3: Run tests — expect compile success, tests pass**

Run: `cargo test -p vcli-core --lib geom`
Expected: 6 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/lib.rs crates/vcli-core/src/geom.rs
git commit -m "vcli-core: Point + Rect with center / top_left / contains"
```

---

### Task 1.2: ProgramId

**Files:**
- Create: `crates/vcli-core/src/ids.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod ids;` and re-export to `lib.rs`**

Append to `lib.rs`:

```rust
pub mod ids;

pub use ids::ProgramId;
```

- [ ] **Step 2: Write test + impl in `ids.rs`**

```rust
//! Identifiers. `ProgramId` is a UUID wrapper — distinct nominal type
//! avoids mixing arbitrary UUIDs with program ids elsewhere.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Unique identifier for a program. Generated by the daemon on submit if the
/// client did not provide one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProgramId(Uuid);

impl ProgramId {
    /// Generate a fresh random v4 program id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Expose the underlying UUID. Useful for logging only — prefer `Display`.
    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ProgramId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProgramId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Error parsing a `ProgramId` from a string.
#[derive(Debug, Error)]
#[error("invalid program id: {0}")]
pub struct ProgramIdParseError(#[from] uuid::Error);

impl FromStr for ProgramId {
    type Err = ProgramIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_ids_are_unique() {
        let a = ProgramId::new();
        let b = ProgramId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn display_parse_roundtrip() {
        let a = ProgramId::new();
        let s = a.to_string();
        let b: ProgramId = s.parse().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn parse_rejects_garbage() {
        let e = "not a uuid".parse::<ProgramId>();
        assert!(e.is_err());
    }

    #[test]
    fn serde_transparent_wraps_uuid() {
        let a = ProgramId::new();
        let j = serde_json::to_string(&a).unwrap();
        // Transparent: serialized as a bare string, not as {"0": "..."}
        assert!(j.starts_with('"') && j.ends_with('"'), "got: {j}");
        let back: ProgramId = serde_json::from_str(&j).unwrap();
        assert_eq!(back, a);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib ids`
Expected: 4 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/ids.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: ProgramId (UUID wrapper with Display/FromStr/serde)"
```

---

### Task 1.3: Frame

**Files:**
- Create: `crates/vcli-core/src/frame.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod frame;` and re-export**

Append to `lib.rs`:

```rust
pub mod frame;

pub use frame::{Frame, FrameFormat};
```

- [ ] **Step 2: Write `frame.rs`**

```rust
//! `Frame` — one capture result passed to perception. Always in logical-pixel
//! resolution (Decision F1 / 4.3). Not serialized (frames are never persisted).

use std::sync::Arc;

use crate::geom::Rect;

/// Pixel format of a frame buffer. v0 emits BGRA8 from macOS ScreenCaptureKit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    /// 4 bytes per pixel, order B, G, R, A. Stride may include row padding.
    Bgra8,
    /// 4 bytes per pixel, order R, G, B, A.
    Rgba8,
}

impl FrameFormat {
    /// Bytes per pixel for this format.
    #[must_use]
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgra8 | Self::Rgba8 => 4,
        }
    }
}

/// A captured screen frame. Shared via `Arc<Frame>` across the tick's perception work.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Pixel format.
    pub format: FrameFormat,
    /// Bounds of the frame in logical pixels. `bounds.top_left()` is where
    /// this buffer originates on the logical desktop.
    pub bounds: Rect,
    /// Row stride in bytes. Usually `bounds.w * bytes_per_pixel`, but some
    /// backends add padding.
    pub stride: usize,
    /// Raw pixel bytes. Length ≥ `stride * bounds.h`.
    pub pixels: Arc<[u8]>,
    /// Monotonic timestamp in nanoseconds since an unspecified epoch.
    pub captured_at_ns: u64,
}

impl Frame {
    /// Convenience constructor. Panics if `pixels.len() < stride * bounds.h`.
    #[must_use]
    pub fn new(
        format: FrameFormat,
        bounds: Rect,
        stride: usize,
        pixels: Arc<[u8]>,
        captured_at_ns: u64,
    ) -> Self {
        let needed = stride.saturating_mul(bounds.h as usize);
        assert!(
            pixels.len() >= needed,
            "frame buffer too small: have {}, need {needed}",
            pixels.len()
        );
        Self { format, bounds, stride, pixels, captured_at_ns }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> i32 {
        self.bounds.w
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> i32 {
        self.bounds.h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frame {
        Frame::new(
            FrameFormat::Bgra8,
            Rect { x: 0, y: 0, w: 4, h: 2 },
            4 * 4,
            vec![0u8; 4 * 4 * 2].into(),
            123,
        )
    }

    #[test]
    fn bytes_per_pixel_matches_format() {
        assert_eq!(FrameFormat::Bgra8.bytes_per_pixel(), 4);
        assert_eq!(FrameFormat::Rgba8.bytes_per_pixel(), 4);
    }

    #[test]
    fn new_stores_inputs_verbatim() {
        let f = sample();
        assert_eq!(f.width(), 4);
        assert_eq!(f.height(), 2);
        assert_eq!(f.stride, 16);
        assert_eq!(f.format, FrameFormat::Bgra8);
        assert_eq!(f.captured_at_ns, 123);
    }

    #[test]
    #[should_panic(expected = "frame buffer too small")]
    fn new_panics_on_too_small_buffer() {
        let _ = Frame::new(
            FrameFormat::Bgra8,
            Rect { x: 0, y: 0, w: 100, h: 100 },
            400,
            vec![0u8; 10].into(),
            0,
        );
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib frame`
Expected: 3 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/frame.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: Frame + FrameFormat (logical pixels, Arc-backed)"
```

---

### Task 1.4: Clock trait + SystemClock + TestClock

**Files:**
- Create: `crates/vcli-core/src/clock.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod clock;` + re-export**

Append to `lib.rs`:

```rust
pub mod clock;

pub use clock::{Clock, SystemClock, TestClock, UnixMs};
```

- [ ] **Step 2: Write `clock.rs`**

```rust
//! Clock abstraction. Prod = `SystemClock`; tests = `TestClock` for determinism.
//!
//! Per Decision 1.6 every time-reading site in vcli takes `&dyn Clock` (or a
//! generic `C: Clock`). Scheduler throttles, elapsed-since-true, and timeouts
//! all resolve via this trait so `TestClock::advance_by(…)` drives them
//! deterministically in tests.
//!
//! Wall-clock / timezone reads belong to a separate `WallClock` trait — NOT
//! in v0 (see TODOS.md "WallClock trait + on_schedule trigger").

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Unix milliseconds since epoch. Used for event timestamps and SQLite rows.
pub type UnixMs = i64;

/// Monotonic time source. Never goes backwards. The returned `Duration` is
/// from an arbitrary fixed epoch — only differences are meaningful.
pub trait Clock: Send + Sync {
    /// Monotonic time reading.
    fn now(&self) -> Duration;

    /// Wall-clock reading for event timestamps.
    /// Implementations should use the same logical "now" for both calls when
    /// possible, but drift between the two is acceptable.
    fn unix_ms(&self) -> UnixMs;
}

/// Production clock. Backed by `std::time::Instant` + `SystemTime::now`.
#[derive(Debug)]
pub struct SystemClock {
    epoch: Instant,
}

impl SystemClock {
    /// Create a new clock whose `now()` reads relative to process start.
    #[must_use]
    pub fn new() -> Self {
        Self { epoch: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    fn unix_ms(&self) -> UnixMs {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        i64::try_from(now.as_millis()).unwrap_or(i64::MAX)
    }
}

/// Deterministic test clock. Both `now()` and `unix_ms()` advance by the same
/// amount when `advance_by` is called.
#[derive(Debug)]
pub struct TestClock {
    inner: Mutex<TestClockInner>,
}

#[derive(Debug)]
struct TestClockInner {
    now: Duration,
    unix_ms: UnixMs,
}

impl TestClock {
    /// Create a clock at `Duration::ZERO` monotonic and at the given unix-ms baseline.
    #[must_use]
    pub fn at_unix_ms(baseline: UnixMs) -> Self {
        Self {
            inner: Mutex::new(TestClockInner { now: Duration::ZERO, unix_ms: baseline }),
        }
    }

    /// Advance both clocks by `d`.
    pub fn advance_by(&self, d: Duration) {
        let mut g = self.inner.lock().unwrap();
        g.now = g.now.saturating_add(d);
        let add = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
        g.unix_ms = g.unix_ms.saturating_add(add);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.inner.lock().unwrap().now
    }

    fn unix_ms(&self) -> UnixMs {
        self.inner.lock().unwrap().unix_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_monotonic_advance() {
        let c = SystemClock::new();
        let a = c.now();
        std::thread::sleep(Duration::from_millis(5));
        let b = c.now();
        assert!(b >= a, "clock went backwards: a={a:?}, b={b:?}");
    }

    #[test]
    fn test_clock_starts_at_zero_monotonic() {
        let c = TestClock::at_unix_ms(1_700_000_000_000);
        assert_eq!(c.now(), Duration::ZERO);
        assert_eq!(c.unix_ms(), 1_700_000_000_000);
    }

    #[test]
    fn test_clock_advance_moves_both_readings() {
        let c = TestClock::at_unix_ms(0);
        c.advance_by(Duration::from_secs(1));
        assert_eq!(c.now(), Duration::from_secs(1));
        assert_eq!(c.unix_ms(), 1_000);
        c.advance_by(Duration::from_millis(250));
        assert_eq!(c.now(), Duration::from_millis(1_250));
        assert_eq!(c.unix_ms(), 1_250);
    }

    #[test]
    fn test_clock_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TestClock>();
        assert_send_sync::<SystemClock>();
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib clock`
Expected: 4 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/clock.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: Clock trait + SystemClock + TestClock"
```

---

### Task 1.5: Region

**Files:**
- Create: `crates/vcli-core/src/region.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod region;` + re-export**

Append to `lib.rs`:

```rust
pub mod region;

pub use region::{Anchor, Region, WindowIndex};
```

- [ ] **Step 2: Write `region.rs`**

```rust
//! Region kinds from the DSL. See spec §DSL → Region kinds.

use serde::{Deserialize, Serialize};

use crate::geom::Rect;

/// Where a `relative_to` region anchors. v0 only supports `match` (the
/// referenced predicate's match box). Reserved for expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// Anchor at the referenced predicate's `match` rectangle.
    Match,
}

/// 0-based window index when `app`/`title_contains` matches multiple windows.
/// When omitted, Decision F2 resolves to the oldest window (lowest AX id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowIndex(pub u32);

/// A region of the screen a predicate targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Region {
    /// Fixed pixel box in logical coordinates.
    Absolute {
        /// Rectangle in logical pixels.
        #[serde(rename = "box")]
        rect: Rect,
    },
    /// Window matching the given app + substring title. Resolved each tick via
    /// the macOS Accessibility API.
    Window {
        /// App name (e.g. "Safari"). Matches `NSRunningApplication.localizedName`.
        app: String,
        /// Substring that must appear in the window title. `None` means any title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title_contains: Option<String>,
        /// Select the Nth matching window (0-based). Omitted = oldest (F2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_index: Option<WindowIndex>,
    },
    /// Region derived from another predicate's match + offset/size.
    RelativeTo {
        /// Name of the referenced predicate in the same program.
        predicate: String,
        /// Anchor in the referenced predicate's match (v0: always `match`).
        #[serde(default = "default_anchor")]
        anchor: Anchor,
        /// Offset added to the anchor point. Logical pixels.
        #[serde(default = "default_offset")]
        offset: crate::geom::Point,
        /// Resulting region size. If omitted, consumers use the referenced
        /// predicate's match size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<Size>,
    },
}

fn default_anchor() -> Anchor {
    Anchor::Match
}

fn default_offset() -> crate::geom::Point {
    crate::geom::Point { x: 0, y: 0 }
}

/// A width/height pair for `relative_to` sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Point, Rect};

    #[test]
    fn absolute_roundtrip() {
        let r = Region::Absolute {
            rect: Rect { x: 0, y: 0, w: 100, h: 50 },
        };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(j, r#"{"kind":"absolute","box":{"x":0,"y":0,"w":100,"h":50}}"#);
        let back: Region = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn window_minimal_roundtrip() {
        let r = Region::Window {
            app: "Safari".into(),
            title_contains: Some("YouTube".into()),
            window_index: None,
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: Region = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn window_omits_none_fields() {
        let r = Region::Window {
            app: "Finder".into(),
            title_contains: None,
            window_index: None,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(j, r#"{"kind":"window","app":"Finder"}"#);
    }

    #[test]
    fn window_index_roundtrip() {
        let r = Region::Window {
            app: "Terminal".into(),
            title_contains: None,
            window_index: Some(WindowIndex(2)),
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: Region = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
        assert!(j.contains(r#""window_index":2"#));
    }

    #[test]
    fn relative_to_with_defaults() {
        let j = r#"{"kind":"relative_to","predicate":"x"}"#;
        let r: Region = serde_json::from_str(j).unwrap();
        match r {
            Region::RelativeTo { predicate, anchor, offset, size } => {
                assert_eq!(predicate, "x");
                assert_eq!(anchor, Anchor::Match);
                assert_eq!(offset, Point { x: 0, y: 0 });
                assert_eq!(size, None);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn relative_to_full_form_roundtrip() {
        let r = Region::RelativeTo {
            predicate: "on_cart".into(),
            anchor: Anchor::Match,
            offset: Point { x: 0, y: 40 },
            size: Some(Size { w: 300, h: 120 }),
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: Region = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn unknown_kind_fails_to_parse() {
        let j = r#"{"kind":"monitor_index","index":0}"#;
        let r: Result<Region, _> = serde_json::from_str(j);
        assert!(r.is_err());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib region`
Expected: 7 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/region.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: Region (absolute | window | relative_to) + WindowIndex"
```

---

### Task 1.6: Predicate + PredicateResult + MatchData

**Files:**
- Create: `crates/vcli-core/src/predicate.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod predicate;` + re-export**

Append to `lib.rs`:

```rust
pub mod predicate;

pub use predicate::{MatchData, Predicate, PredicateKind, PredicateResult};
```

- [ ] **Step 2: Write `predicate.rs`**

```rust
//! Predicate definitions and evaluation results.
//!
//! Each `Predicate` kind maps to a `PredicateEvaluator` impl (defined in
//! `vcli-perception`). `PredicateResult` is what evaluators return and what
//! the cache stores.

use serde::{Deserialize, Serialize};

use crate::clock::UnixMs;
use crate::geom::Rect;
use crate::region::Region;

/// Confidence as a `f32` in [0, 1]. Newtyped to avoid silent mismatches with
/// arbitrary f32s and to prevent accidental equality checks that surprise.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Confidence(pub f32);

/// RGB triple for color matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rgb(pub [u8; 3]);

/// Predicate kinds supported in v0.
///
/// Post-v0 kinds (`ocr`, `ocr_text`, `vlm`) plug in via the same evaluator
/// trait without DSL schema churn — they'll appear as additional variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PredicateKind {
    /// Template match against a named image asset within a region.
    Template {
        /// Either a file path resolved at submit, or `sha256:<hex>` once
        /// rewritten by the daemon's submit module (Decision F4).
        image: String,
        /// Required confidence for truthy. Inclusive.
        confidence: Confidence,
        /// Where on screen to search.
        region: Region,
        /// Minimum milliseconds between evaluations (Tier 2).
        #[serde(default = "default_template_throttle_ms")]
        throttle_ms: u32,
    },
    /// Single-pixel color match.
    ColorAt {
        /// Logical-pixel coordinate.
        point: crate::geom::Point,
        /// Expected RGB.
        rgb: Rgb,
        /// Max Euclidean RGB distance considered a match.
        tolerance: u16,
    },
    /// Perceptual-hash comparison of a region against a baseline.
    PixelDiff {
        /// Region to hash.
        region: Region,
        /// `sha256:<hex>` baseline asset reference (stored as the image's
        /// content hash; the evaluator rehashes the region into a perceptual
        /// hash at eval time and compares).
        baseline: String,
        /// Fractional Hamming-distance threshold (0.0..=1.0).
        threshold: f32,
    },
    /// Logical AND across named predicates.
    AllOf {
        /// Names of predicates in the same program.
        of: Vec<String>,
    },
    /// Logical OR across named predicates.
    AnyOf {
        /// Names of predicates in the same program.
        of: Vec<String>,
    },
    /// Logical negation.
    Not {
        /// Predicate name in the same program.
        of: String,
    },
    /// True when the referenced predicate has been continuously truthy for at
    /// least `ms` milliseconds. Per-program state.
    ElapsedMsSinceTrue {
        /// Predicate name in the same program.
        predicate: String,
        /// Threshold in milliseconds.
        ms: u32,
    },
}

fn default_template_throttle_ms() -> u32 {
    200
}

/// Full predicate definition: the kind plus optional cross-program controls
/// carried in future extensions (v0 has none).
pub type Predicate = PredicateKind;

/// What an evaluator returns and what the `PerceptionCache` stores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredicateResult {
    /// Whether the predicate's truth condition held at eval time.
    pub truthy: bool,
    /// Location/confidence data, if the kind produces one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_data: Option<MatchData>,
    /// Unix ms when this result was computed.
    pub at: UnixMs,
}

/// Match metadata for location-producing predicate kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchData {
    /// Match bounding box in logical pixels (absolute screen coords).
    #[serde(rename = "box")]
    pub bbox: Rect,
    /// Reported confidence from the evaluator.
    pub confidence: Confidence,
}

impl MatchData {
    /// Convenience — bounding box center (matches `$p.match.center` in DSL
    /// expressions).
    #[must_use]
    pub fn center(&self) -> crate::geom::Point {
        self.bbox.center()
    }

    /// Convenience — top-left corner.
    #[must_use]
    pub fn top_left(&self) -> crate::geom::Point {
        self.bbox.top_left()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Point, Rect};
    use crate::region::Region;

    #[test]
    fn template_kind_roundtrip_with_default_throttle() {
        let j = r#"{"kind":"template","image":"assets/x.png","confidence":0.9,
                    "region":{"kind":"absolute","box":{"x":0,"y":0,"w":10,"h":10}}}"#;
        let p: PredicateKind = serde_json::from_str(j).unwrap();
        match &p {
            PredicateKind::Template { throttle_ms, .. } => assert_eq!(*throttle_ms, 200),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn color_at_roundtrip() {
        let p = PredicateKind::ColorAt {
            point: Point { x: 10, y: 20 },
            rgb: Rgb([255, 0, 128]),
            tolerance: 15,
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: PredicateKind = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn pixel_diff_roundtrip() {
        let p = PredicateKind::PixelDiff {
            region: Region::Absolute { rect: Rect { x: 0, y: 0, w: 50, h: 50 } },
            baseline: "sha256:abcd".into(),
            threshold: 0.05,
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: PredicateKind = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn all_of_and_any_of_and_not_roundtrips() {
        for p in [
            PredicateKind::AllOf { of: vec!["a".into(), "b".into()] },
            PredicateKind::AnyOf { of: vec!["a".into()] },
            PredicateKind::Not { of: "a".into() },
        ] {
            let j = serde_json::to_string(&p).unwrap();
            let back: PredicateKind = serde_json::from_str(&j).unwrap();
            assert_eq!(back, p);
        }
    }

    #[test]
    fn elapsed_ms_since_true_roundtrip() {
        let p = PredicateKind::ElapsedMsSinceTrue {
            predicate: "visible".into(),
            ms: 500,
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: PredicateKind = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn match_data_center_and_top_left() {
        let m = MatchData {
            bbox: Rect { x: 10, y: 10, w: 40, h: 20 },
            confidence: Confidence(0.95),
        };
        assert_eq!(m.center(), Point { x: 30, y: 20 });
        assert_eq!(m.top_left(), Point { x: 10, y: 10 });
    }

    #[test]
    fn predicate_result_no_match_serializes_without_match_field() {
        let r = PredicateResult { truthy: false, match_data: None, at: 1 };
        let j = serde_json::to_string(&r).unwrap();
        assert!(!j.contains("match"), "got {j}");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib predicate`
Expected: 7 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/predicate.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: Predicate / PredicateKind / PredicateResult / MatchData"
```

---

### Task 1.7: InputAction + Button + Modifier

**Files:**
- Create: `crates/vcli-core/src/action.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod action;` + re-export**

Append to `lib.rs`:

```rust
pub mod action;

pub use action::{Button, InputAction, Modifier};
```

- [ ] **Step 2: Write `action.rs`**

```rust
//! Low-level input actions the `Input` trait dispatches. Distinct from DSL
//! `Step` because steps can carry expressions (e.g. `$p.match.center`) that
//! have to be resolved to concrete points before dispatch.

use serde::{Deserialize, Serialize};

use crate::geom::Point;

/// Mouse buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    /// Left mouse button.
    Left,
    /// Right mouse button.
    Right,
    /// Middle mouse button.
    Middle,
}

/// Keyboard modifier keys. Distinct from regular keys because a `Key` action
/// can carry a `Vec<Modifier>` for chords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    /// Command (macOS) / Super.
    Cmd,
    /// Shift.
    Shift,
    /// Option (macOS) / Alt.
    Alt,
    /// Control.
    Ctrl,
}

/// Resolved input action — all expressions already substituted to concrete
/// points/strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputAction {
    /// Move cursor (no click).
    Move {
        /// Destination in logical pixels.
        at: Point,
    },
    /// Click at a point with a given button.
    Click {
        /// Point to click.
        at: Point,
        /// Which mouse button.
        button: Button,
    },
    /// Type literal text (via keyboard events; respects active layout).
    Type {
        /// Text to type.
        text: String,
    },
    /// Press a key chord (one non-modifier key plus zero or more modifiers).
    Key {
        /// Key name using the vcli canonical set (e.g. "s", "return", "space").
        key: String,
        /// Held modifiers during the press.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        modifiers: Vec<Modifier>,
    },
    /// Scroll at a point by (dx, dy) in logical pixels.
    Scroll {
        /// Point to scroll over.
        at: Point,
        /// Horizontal delta (right is positive).
        #[serde(default)]
        dx: i32,
        /// Vertical delta (down is positive).
        #[serde(default)]
        dy: i32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_roundtrip() {
        let a = InputAction::Click { at: Point { x: 10, y: 20 }, button: Button::Left };
        let j = serde_json::to_string(&a).unwrap();
        assert!(j.contains(r#""kind":"click""#));
        assert!(j.contains(r#""button":"left""#));
        let back: InputAction = serde_json::from_str(&j).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn type_roundtrip() {
        let a = InputAction::Type { text: "hello".into() };
        let back: InputAction = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn key_chord_roundtrip() {
        let a = InputAction::Key {
            key: "s".into(),
            modifiers: vec![Modifier::Cmd, Modifier::Shift],
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: InputAction = serde_json::from_str(&j).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn key_without_modifiers_omits_field() {
        let a = InputAction::Key { key: "return".into(), modifiers: vec![] };
        let j = serde_json::to_string(&a).unwrap();
        assert!(!j.contains("modifiers"));
    }

    #[test]
    fn scroll_uses_default_zero_axes() {
        let j = r#"{"kind":"scroll","at":{"x":0,"y":0},"dy":-40}"#;
        let a: InputAction = serde_json::from_str(j).unwrap();
        match a {
            InputAction::Scroll { dx, dy, .. } => {
                assert_eq!(dx, 0);
                assert_eq!(dy, -40);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn move_roundtrip() {
        let a = InputAction::Move { at: Point { x: 5, y: 5 } };
        let back: InputAction = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib action`
Expected: 6 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/action.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: InputAction (move | click | type | key | scroll)"
```

---

### Task 1.8: Step

**Files:**
- Create: `crates/vcli-core/src/step.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod step;` + re-export**

Append to `lib.rs`:

```rust
pub mod step;

pub use step::{OnFail, OnTimeout, Step, Target};
```

- [ ] **Step 2: Write `step.rs`**

```rust
//! DSL `Step` — the vocabulary shared by `watches[*].do` and `body`.
//! Inputs carry resolvable expressions (`$pred.match.center`) rather than
//! concrete points — that resolution happens in `vcli-runtime` before
//! producing an `InputAction` for dispatch.

use serde::{Deserialize, Serialize};

use crate::action::{Button, Modifier};

/// Target of a step that interacts with a screen location. Either a concrete
/// point (for absolute coordinates) or an expression string like
/// `"$skip_visible.match.center"` resolved at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    /// Absolute point. `{"x": 100, "y": 200}`.
    Absolute(crate::geom::Point),
    /// Expression. `"$p.match.center"`.
    Expression(String),
}

/// What happens when a `wait_for` step's predicate never becomes truthy
/// before `timeout_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnTimeout {
    /// Program transitions to `failed`.
    Fail,
    /// Skip the wait and continue to the next body step.
    Continue,
    /// Re-evaluate the predicate one more tick. (v0: equivalent to one extra tick; no backoff.)
    Retry,
}

/// What happens when an `assert` fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFail {
    /// Program transitions to `failed`.
    Fail,
    /// Skip and continue to the next body step. Useful for best-effort checks.
    Continue,
}

/// A DSL step. Used in both `body` (sequential) and `watches[*].do` (reactive).
/// Control-flow variants (`WaitFor`, `Assert`, `SleepMs`) are body-only; the
/// validator (`vcli-dsl`) rejects them in watches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    /// Move cursor.
    Move {
        /// Destination.
        at: Target,
    },
    /// Click at a target.
    Click {
        /// Click target.
        at: Target,
        /// Which button to click with.
        #[serde(default = "default_button")]
        button: Button,
    },
    /// Type literal text.
    Type {
        /// Text.
        text: String,
    },
    /// Press a key combo.
    Key {
        /// Key name.
        key: String,
        /// Modifier keys held during the press.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        modifiers: Vec<Modifier>,
    },
    /// Scroll at a target.
    Scroll {
        /// Scroll target.
        at: Target,
        /// Horizontal delta.
        #[serde(default)]
        dx: i32,
        /// Vertical delta.
        #[serde(default)]
        dy: i32,
    },

    /// Body-only. Block until predicate becomes truthy or timeout fires.
    WaitFor {
        /// Predicate name.
        predicate: String,
        /// Max milliseconds to wait.
        timeout_ms: u32,
        /// Behavior on timeout.
        #[serde(default = "default_on_timeout")]
        on_timeout: OnTimeout,
    },
    /// Body-only. Fail the program (or continue) if the named predicate is not truthy.
    Assert {
        /// Predicate name.
        predicate: String,
        /// Behavior on failure.
        #[serde(default = "default_on_fail")]
        on_fail: OnFail,
    },
    /// Body-only. Sleep for a fixed duration. NOT resumable (see Decision C).
    SleepMs {
        /// Milliseconds.
        ms: u32,
    },
}

fn default_button() -> Button {
    Button::Left
}
fn default_on_timeout() -> OnTimeout {
    OnTimeout::Fail
}
fn default_on_fail() -> OnFail {
    OnFail::Fail
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;

    #[test]
    fn click_with_expression_target() {
        let j = r#"{"kind":"click","at":"$skip.match.center"}"#;
        let s: Step = serde_json::from_str(j).unwrap();
        assert_eq!(
            s,
            Step::Click {
                at: Target::Expression("$skip.match.center".into()),
                button: Button::Left,
            }
        );
    }

    #[test]
    fn click_with_absolute_target() {
        let j = r#"{"kind":"click","at":{"x":10,"y":20}}"#;
        let s: Step = serde_json::from_str(j).unwrap();
        assert_eq!(
            s,
            Step::Click {
                at: Target::Absolute(Point { x: 10, y: 20 }),
                button: Button::Left,
            }
        );
    }

    #[test]
    fn click_button_roundtrip() {
        let s = Step::Click {
            at: Target::Expression("$p.match.center".into()),
            button: Button::Right,
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: Step = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn wait_for_defaults_to_fail_on_timeout() {
        let j = r#"{"kind":"wait_for","predicate":"p","timeout_ms":1000}"#;
        let s: Step = serde_json::from_str(j).unwrap();
        match s {
            Step::WaitFor { on_timeout, .. } => assert_eq!(on_timeout, OnTimeout::Fail),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn assert_defaults_to_fail_on_fail() {
        let j = r#"{"kind":"assert","predicate":"p"}"#;
        let s: Step = serde_json::from_str(j).unwrap();
        match s {
            Step::Assert { on_fail, .. } => assert_eq!(on_fail, OnFail::Fail),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn sleep_ms_roundtrip() {
        let s = Step::SleepMs { ms: 250 };
        let j = serde_json::to_string(&s).unwrap();
        let back: Step = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn type_and_key_and_scroll_roundtrips() {
        for s in [
            Step::Type { text: "hi".into() },
            Step::Key { key: "s".into(), modifiers: vec![Modifier::Cmd] },
            Step::Scroll { at: Target::Expression("$p.match.center".into()), dx: 0, dy: -40 },
        ] {
            let back: Step = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            assert_eq!(back, s);
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib step`
Expected: 7 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/step.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: Step + Target + OnTimeout + OnFail"
```

---

### Task 1.9: Watch + Lifetime

**Files:**
- Create: `crates/vcli-core/src/watch.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod watch;` + re-export**

Append to `lib.rs`:

```rust
pub mod watch;

pub use watch::{Lifetime, Watch, WatchWhen};
```

- [ ] **Step 2: Write `watch.rs`**

```rust
//! Reactive watches — `when → do` with a lifetime.
//!
//! `when` can be either a predicate name (referencing a named entry in the
//! program's `predicates` map) or an inline anonymous predicate. The DSL
//! validator checks name references; inline predicates are validated in
//! place.

use serde::{Deserialize, Serialize};

use crate::predicate::PredicateKind;
use crate::step::Step;

/// Named-or-inline predicate reference for `watch.when`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WatchWhen {
    /// Reference by name. The name must exist in the enclosing program's
    /// `predicates` map (validated in `vcli-dsl`).
    ByName(String),
    /// Inline anonymous predicate.
    Inline(Box<PredicateKind>),
}

/// How long a watch stays active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Lifetime {
    /// Fires exactly once when `when` transitions false→true, then is removed.
    OneShot,
    /// Fires every false→true transition, respecting `throttle_ms`.
    Persistent,
    /// Persistent until the named predicate becomes truthy.
    UntilPredicate {
        /// Predicate name that, when truthy, removes this watch.
        name: String,
    },
    /// Persistent until N milliseconds after the program started `running`.
    TimeoutMs {
        /// Duration in ms from `running` entry.
        ms: u32,
    },
}

/// A reactive rule on a program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watch {
    /// Truth condition.
    pub when: WatchWhen,
    /// Steps to run on each fire. Only input Steps are valid here (validator
    /// rejects `wait_for` / `assert` / `sleep_ms` inside watches).
    #[serde(rename = "do")]
    pub steps: Vec<Step>,
    /// Minimum ms between fires. Defaults to 0 (no throttle).
    #[serde(default)]
    pub throttle_ms: u32,
    /// Persistence policy. Defaults to `persistent`.
    #[serde(default = "default_lifetime")]
    pub lifetime: Lifetime,
}

fn default_lifetime() -> Lifetime {
    Lifetime::Persistent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Button;
    use crate::step::Target;

    #[test]
    fn by_name_watch_roundtrip() {
        let w = Watch {
            when: WatchWhen::ByName("skip_visible".into()),
            steps: vec![Step::Click {
                at: Target::Expression("$skip_visible.match.center".into()),
                button: Button::Left,
            }],
            throttle_ms: 500,
            lifetime: Lifetime::Persistent,
        };
        let j = serde_json::to_string(&w).unwrap();
        let back: Watch = serde_json::from_str(&j).unwrap();
        assert_eq!(back, w);
    }

    #[test]
    fn inline_predicate_watch_parses() {
        let j = r#"{
            "when": {"kind":"color_at","point":{"x":0,"y":0},"rgb":[0,0,0],"tolerance":10},
            "do": [{"kind":"move","at":{"x":0,"y":0}}]
        }"#;
        let w: Watch = serde_json::from_str(j).unwrap();
        matches!(w.when, WatchWhen::Inline(_));
        assert_eq!(w.throttle_ms, 0);
        assert_eq!(w.lifetime, Lifetime::Persistent);
    }

    #[test]
    fn lifetime_variants_roundtrip() {
        for l in [
            Lifetime::OneShot,
            Lifetime::Persistent,
            Lifetime::UntilPredicate { name: "done".into() },
            Lifetime::TimeoutMs { ms: 30_000 },
        ] {
            let j = serde_json::to_string(&l).unwrap();
            let back: Lifetime = serde_json::from_str(&j).unwrap();
            assert_eq!(back, l);
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib watch`
Expected: 3 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/watch.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: Watch + WatchWhen + Lifetime"
```

---

### Task 1.10: Trigger

**Files:**
- Create: `crates/vcli-core/src/trigger.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod trigger;` + re-export**

Append to `lib.rs`:

```rust
pub mod trigger;

pub use trigger::Trigger;
```

- [ ] **Step 2: Write `trigger.rs`**

```rust
//! Program start triggers. `on_schedule` is deliberately absent in v0 — it
//! requires a `WallClock` trait (see TODOS.md) and lands post-v0.

use serde::{Deserialize, Serialize};

/// How a program transitions from `waiting` into `running`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Fire immediately once the daemon is ready and the program has been loaded.
    OnSubmit,
    /// Fire when the named predicate becomes truthy.
    OnPredicate {
        /// Predicate name in the same program.
        name: String,
    },
    /// Stay in `waiting` until `vcli start <id>`.
    Manual,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_submit_roundtrip() {
        let t = Trigger::OnSubmit;
        let j = serde_json::to_string(&t).unwrap();
        assert_eq!(j, r#"{"kind":"on_submit"}"#);
        let back: Trigger = serde_json::from_str(&j).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn on_predicate_roundtrip() {
        let t = Trigger::OnPredicate { name: "ready".into() };
        let j = serde_json::to_string(&t).unwrap();
        let back: Trigger = serde_json::from_str(&j).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn manual_roundtrip() {
        let t = Trigger::Manual;
        let back: Trigger = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn on_schedule_is_rejected() {
        // Decision D removed on_schedule from v0. Defensive: serde shouldn't accept it.
        let j = r#"{"kind":"on_schedule","cron":"0 21 * * *"}"#;
        let r: Result<Trigger, _> = serde_json::from_str(j);
        assert!(r.is_err(), "on_schedule must not parse in v0");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib trigger`
Expected: 4 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/trigger.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: Trigger (on_submit | on_predicate | manual)"
```

---

### Task 1.11: ProgramState

**Files:**
- Create: `crates/vcli-core/src/state.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod state;` + re-export**

Append to `lib.rs`:

```rust
pub mod state;

pub use state::ProgramState;
```

- [ ] **Step 2: Write `state.rs`**

```rust
//! Program lifecycle state. See spec §Runtime → Program state machine.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Lifecycle state of a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramState {
    /// Submitted but daemon not yet ready / program not yet loaded into scheduler.
    Pending,
    /// Loaded; waiting for trigger to fire.
    Waiting,
    /// Trigger fired; scheduler is advancing body + watches.
    Running,
    /// Reserved — no v0 transition enters this.
    Blocked,
    /// Body complete (non-empty body) or last watch removed (pure-watches programs).
    Completed,
    /// Body error, assert failure, timeout, `capture_failed`, or `daemon_restart`.
    Failed,
    /// Explicit `vcli cancel`.
    Cancelled,
}

impl ProgramState {
    /// Whether this is a terminal state (no further transitions).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Whether this state represents active execution.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, Self::Waiting | Self::Running | Self::Blocked)
    }

    /// Canonical string form (same as serde snake_case).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for ProgramState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error parsing a `ProgramState` from a string.
#[derive(Debug, Error)]
#[error("unknown program state: {0}")]
pub struct ProgramStateParseError(pub String);

impl FromStr for ProgramState {
    type Err = ProgramStateParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "pending" => Self::Pending,
            "waiting" => Self::Waiting,
            "running" => Self::Running,
            "blocked" => Self::Blocked,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            other => return Err(ProgramStateParseError(other.to_string())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_classification() {
        assert!(ProgramState::Completed.is_terminal());
        assert!(ProgramState::Failed.is_terminal());
        assert!(ProgramState::Cancelled.is_terminal());
        assert!(!ProgramState::Running.is_terminal());
        assert!(!ProgramState::Waiting.is_terminal());
    }

    #[test]
    fn active_classification() {
        assert!(ProgramState::Waiting.is_active());
        assert!(ProgramState::Running.is_active());
        assert!(ProgramState::Blocked.is_active());
        assert!(!ProgramState::Pending.is_active());
        assert!(!ProgramState::Completed.is_active());
    }

    #[test]
    fn display_parse_roundtrip() {
        for s in [
            ProgramState::Pending,
            ProgramState::Waiting,
            ProgramState::Running,
            ProgramState::Blocked,
            ProgramState::Completed,
            ProgramState::Failed,
            ProgramState::Cancelled,
        ] {
            let back: ProgramState = s.to_string().parse().unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn serde_snake_case_matches_as_str() {
        let j = serde_json::to_string(&ProgramState::Running).unwrap();
        assert_eq!(j, r#""running""#);
    }

    #[test]
    fn parse_rejects_unknown() {
        let r: Result<ProgramState, _> = "invalid".parse();
        assert!(r.is_err());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib state`
Expected: 5 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/state.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: ProgramState + transitions helpers"
```

---

### Task 1.12: Program (top-level DSL document)

**Files:**
- Create: `crates/vcli-core/src/program.rs`
- Modify: `crates/vcli-core/src/lib.rs`
- Create: `fixtures/yt_ad_skipper.json`

- [ ] **Step 1: Add `pub mod program;` + re-export**

Append to `lib.rs`:

```rust
pub mod program;

pub use program::{DslVersion, OnComplete, OnFail, Priority, Program};
```

- [ ] **Step 2: Write the YT ad skipper fixture**

Create `fixtures/yt_ad_skipper.json` (content copied verbatim from the spec §DSL full example):

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
    }
  },
  "watches": [
    {
      "when": "skip_visible",
      "do": [{ "kind": "click", "at": "$skip_visible.match.center" }],
      "throttle_ms": 500,
      "lifetime": { "kind": "persistent" }
    }
  ],
  "body": [],
  "on_complete": { "emit": "ad_skipped" }
}
```

- [ ] **Step 3: Write `program.rs`**

```rust
//! `Program` — the top-level DSL document. Matches spec §DSL → Program shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::ProgramId;
use crate::predicate::PredicateKind;
use crate::step::Step;
use crate::trigger::Trigger;
use crate::watch::Watch;

/// DSL major version the daemon understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DslVersion(pub String);

impl DslVersion {
    /// Current v0 DSL version.
    pub const V0_1: &'static str = "0.1";

    /// Major digit (everything before the first `.`).
    #[must_use]
    pub fn major(&self) -> &str {
        self.0.split('.').next().unwrap_or("")
    }
}

/// Priority for action arbitration (Decision 1.5). Higher wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Priority(pub i32);

impl Default for Priority {
    fn default() -> Self {
        Self(0)
    }
}

/// Event emitter for program completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnComplete {
    /// Custom event name to emit alongside the system `program.completed` event.
    pub emit: String,
}

/// Event emitter for program failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnFail {
    /// Custom event name to emit alongside the system `program.failed` event.
    pub emit: String,
}

/// Top-level program document.
///
/// Labels and predicate names use `BTreeMap` for deterministic canonical JSON
/// output — see Decision 1.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// DSL version string.
    pub version: DslVersion,
    /// Human label (not unique).
    pub name: String,
    /// Optional client-supplied id. Daemon assigns a fresh UUID when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ProgramId>,
    /// Program start trigger.
    pub trigger: Trigger,
    /// Named predicates.
    #[serde(default)]
    pub predicates: BTreeMap<String, PredicateKind>,
    /// Reactive rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watches: Vec<Watch>,
    /// Sequential body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<Step>,
    /// Optional completion emitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_complete: Option<OnComplete>,
    /// Optional failure emitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_fail: Option<OnFail>,
    /// Program-level timeout. `None` = no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
    /// Free-form tags for filtering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// Priority for arbitration tiebreak (Decision 1.5).
    #[serde(default, skip_serializing_if = "is_default_priority")]
    pub priority: Priority,
}

fn is_default_priority(p: &Priority) -> bool {
    p.0 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const YT_FIXTURE: &str = include_str!("../../../fixtures/yt_ad_skipper.json");

    #[test]
    fn dsl_version_major() {
        assert_eq!(DslVersion("0.1".into()).major(), "0");
        assert_eq!(DslVersion("1.2.3".into()).major(), "1");
    }

    #[test]
    fn yt_ad_skipper_fixture_parses() {
        let p: Program = serde_json::from_str(YT_FIXTURE).expect("fixture must parse");
        assert_eq!(p.name, "yt-ad-skipper");
        assert_eq!(p.version.0, "0.1");
        assert_eq!(p.predicates.len(), 1);
        assert!(p.predicates.contains_key("skip_visible"));
        assert_eq!(p.watches.len(), 1);
        assert!(p.body.is_empty());
        assert_eq!(p.on_complete.as_ref().unwrap().emit, "ad_skipped");
        assert_eq!(p.priority, Priority::default());
    }

    #[test]
    fn yt_ad_skipper_fixture_roundtrips() {
        let p: Program = serde_json::from_str(YT_FIXTURE).unwrap();
        let j = serde_json::to_string(&p).unwrap();
        let p2: Program = serde_json::from_str(&j).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn priority_default_is_omitted_from_serialization() {
        let p = Program {
            version: DslVersion("0.1".into()),
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
            priority: Priority::default(),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(!j.contains("priority"), "default priority must not serialize: {j}");
    }

    #[test]
    fn priority_nonzero_serializes() {
        let p = Program {
            version: DslVersion("0.1".into()),
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
            priority: Priority(5),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""priority":5"#), "got {j}");
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p vcli-core --lib program`
Expected: 5 tests, all passing.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-core/src/program.rs crates/vcli-core/src/lib.rs fixtures/yt_ad_skipper.json
git commit -m "vcli-core: Program + YT ad skipper fixture roundtrip"
```

---

### Task 1.13: Canonical JSON + PredicateHash

**Files:**
- Create: `crates/vcli-core/src/canonical.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod canonical;` + re-export**

Append to `lib.rs`:

```rust
pub mod canonical;

pub use canonical::{canonicalize, predicate_hash, CanonicalError, PredicateHash};
```

- [ ] **Step 2: Write `canonical.rs`**

Canonical form per Decision 1.1: object keys sorted lexicographically (UTF-8 byte order), numbers via `ryu` when fractional / plain-int otherwise, strings normalized to UTF-8 NFC, no whitespace.

```rust
//! Canonical JSON serialization for stable hashing. Decision 1.1.
//!
//! The function `canonicalize` takes any `serde_json::Value` and emits a
//! `Vec<u8>` of canonical bytes. Two semantically-equal values MUST produce
//! identical bytes. `predicate_hash` is `sha256(canonicalize(value))` wrapped
//! in the `PredicateHash` newtype.

use std::fmt;
use std::io::{self, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// Canonicalization error.
#[derive(Debug, Error)]
pub enum CanonicalError {
    /// IO error while writing.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Serialize `value` into canonical bytes.
///
/// Rules:
/// - Object keys sorted lexicographically by UTF-8 bytes.
/// - Numbers: integers as plain decimal (no leading zeros, `-0` normalized to `0`);
///   non-integers via `ryu`.
/// - Strings: UTF-8 NFC-normalized, then JSON-escaped (minimal escapes).
/// - No whitespace anywhere.
///
/// # Errors
///
/// Returns `CanonicalError::Io` only if the underlying writer fails, which
/// cannot happen for the `Vec<u8>` we use internally — but the API surfaces
/// the `Result` for future generalization.
pub fn canonicalize(value: &Value) -> Result<Vec<u8>, CanonicalError> {
    let mut out = Vec::new();
    write_value(&mut out, value)?;
    Ok(out)
}

fn write_value(w: &mut Vec<u8>, v: &Value) -> io::Result<()> {
    match v {
        Value::Null => w.write_all(b"null"),
        Value::Bool(true) => w.write_all(b"true"),
        Value::Bool(false) => w.write_all(b"false"),
        Value::Number(n) => write_number(w, n),
        Value::String(s) => write_string(w, s),
        Value::Array(a) => write_array(w, a),
        Value::Object(m) => write_object(w, m),
    }
}

fn write_number(w: &mut Vec<u8>, n: &serde_json::Number) -> io::Result<()> {
    if let Some(i) = n.as_i64() {
        // Plain decimal. `-0` → `0`.
        let s = if i == 0 { "0".to_string() } else { i.to_string() };
        w.write_all(s.as_bytes())
    } else if let Some(u) = n.as_u64() {
        w.write_all(u.to_string().as_bytes())
    } else if let Some(f) = n.as_f64() {
        if !f.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "canonical JSON: non-finite number",
            ));
        }
        let f_norm = if f == 0.0 { 0.0 } else { f };
        let mut buf = ryu::Buffer::new();
        w.write_all(buf.format(f_norm).as_bytes())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "unrepresentable number"))
    }
}

fn write_string(w: &mut Vec<u8>, s: &str) -> io::Result<()> {
    let normalized: String = s.nfc().collect();
    w.write_all(b"\"")?;
    for ch in normalized.chars() {
        match ch {
            '"' => w.write_all(b"\\\"")?,
            '\\' => w.write_all(b"\\\\")?,
            '\n' => w.write_all(b"\\n")?,
            '\r' => w.write_all(b"\\r")?,
            '\t' => w.write_all(b"\\t")?,
            '\x08' => w.write_all(b"\\b")?,
            '\x0c' => w.write_all(b"\\f")?,
            c if (c as u32) < 0x20 => {
                let s = format!("\\u{:04x}", c as u32);
                w.write_all(s.as_bytes())?;
            }
            c => {
                let mut buf = [0u8; 4];
                w.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    w.write_all(b"\"")
}

fn write_array(w: &mut Vec<u8>, arr: &[Value]) -> io::Result<()> {
    w.write_all(b"[")?;
    for (i, v) in arr.iter().enumerate() {
        if i > 0 {
            w.write_all(b",")?;
        }
        write_value(w, v)?;
    }
    w.write_all(b"]")
}

fn write_object(w: &mut Vec<u8>, obj: &serde_json::Map<String, Value>) -> io::Result<()> {
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    w.write_all(b"{")?;
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            w.write_all(b",")?;
        }
        write_string(w, k)?;
        w.write_all(b":")?;
        write_value(w, obj.get(*k).unwrap())?;
    }
    w.write_all(b"}")
}

/// 32-byte SHA-256 over canonical bytes, hex-encoded.
///
/// We implement SHA-256 manually using a tiny pure-Rust rotate-and-add loop
/// to avoid pulling in a crypto crate just for this. For production use the
/// Rust `sha2` crate is preferable — but this crate is meant to be tiny and
/// dep-light; `sha2` can be introduced later if needed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PredicateHash(String);

impl PredicateHash {
    /// Hex string form (`"sha256:<hex>"` is NOT used here — that prefix lives in
    /// the daemon's asset-reference strings. This type is just the hash bytes).
    #[must_use]
    pub fn hex(&self) -> &str {
        &self.0
    }

    /// Wrap a precomputed hex hash. Caller is responsible for correctness.
    #[must_use]
    pub fn from_hex(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }
}

impl fmt::Display for PredicateHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Hash a `Value` by canonicalizing and SHA-256-ing.
///
/// # Errors
///
/// Returns `CanonicalError::Io` only on writer failure (effectively unreachable).
pub fn predicate_hash(value: &Value) -> Result<PredicateHash, CanonicalError> {
    let bytes = canonicalize(value)?;
    Ok(PredicateHash(sha256_hex(&bytes)))
}

// ---- minimal SHA-256 (FIPS-180-4) ----------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    let mut state: [u32; 8] = [
        0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a,
        0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
    ];
    let k: [u32; 64] = [
        0x428a_2f98,0x7137_4491,0xb5c0_fbcf,0xe9b5_dba5,0x3956_c25b,0x59f1_11f1,0x923f_82a4,0xab1c_5ed5,
        0xd807_aa98,0x1283_5b01,0x2431_85be,0x550c_7dc3,0x72be_5d74,0x80de_b1fe,0x9bdc_06a7,0xc19b_f174,
        0xe49b_69c1,0xefbe_4786,0x0fc1_9dc6,0x240c_a1cc,0x2de9_2c6f,0x4a74_84aa,0x5cb0_a9dc,0x76f9_88da,
        0x983e_5152,0xa831_c66d,0xb003_27c8,0xbf59_7fc7,0xc6e0_0bf3,0xd5a7_9147,0x06ca_6351,0x1429_2967,
        0x27b7_0a85,0x2e1b_2138,0x4d2c_6dfc,0x5338_0d13,0x650a_7354,0x766a_0abb,0x81c2_c92e,0x9272_2c85,
        0xa2bf_e8a1,0xa81a_664b,0xc24b_8b70,0xc76c_51a3,0xd192_e819,0xd699_0624,0xf40e_3585,0x106a_a070,
        0x19a4_c116,0x1e37_6c08,0x2748_774c,0x34b0_bcb5,0x391c_0cb3,0x4ed8_aa4a,0x5b9c_ca4f,0x682e_6ff3,
        0x748f_82ee,0x78a5_636f,0x84c8_7814,0x8cc7_0208,0x90be_fffa,0xa450_6ceb,0xbef9_a3f7,0xc671_78f2,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut h = state;
        for i in 0..64 {
            let s1 = h[4].rotate_right(6) ^ h[4].rotate_right(11) ^ h[4].rotate_right(25);
            let ch = (h[4] & h[5]) ^ (!h[4] & h[6]);
            let temp1 = h[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k[i])
                .wrapping_add(w[i]);
            let s0 = h[0].rotate_right(2) ^ h[0].rotate_right(13) ^ h[0].rotate_right(22);
            let maj = (h[0] & h[1]) ^ (h[0] & h[2]) ^ (h[1] & h[2]);
            let temp2 = s0.wrapping_add(maj);
            h[7] = h[6];
            h[6] = h[5];
            h[5] = h[4];
            h[4] = h[3].wrapping_add(temp1);
            h[3] = h[2];
            h[2] = h[1];
            h[1] = h[0];
            h[0] = temp1.wrapping_add(temp2);
        }
        for (i, val) in h.iter().enumerate() {
            state[i] = state[i].wrapping_add(*val);
        }
    }
    let mut s = String::with_capacity(64);
    for word in state {
        s.push_str(&format!("{word:08x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_sorted_lexicographically() {
        let v = json!({"b": 1, "a": 2, "c": {"z": 1, "a": 2}});
        let out = canonicalize(&v).unwrap();
        assert_eq!(out, br#"{"a":2,"b":1,"c":{"a":2,"z":1}}"#);
    }

    #[test]
    fn integers_as_plain_decimal() {
        let v = json!({"n": 42, "z": 0, "neg": -7});
        let out = canonicalize(&v).unwrap();
        assert_eq!(out, br#"{"n":42,"neg":-7,"z":0}"#);
    }

    #[test]
    fn floats_via_ryu() {
        let v = json!({"x": 0.1});
        let out = canonicalize(&v).unwrap();
        // ryu emits shortest round-trip form.
        assert_eq!(out, br#"{"x":0.1}"#);
    }

    #[test]
    fn empty_containers_emit_correctly() {
        assert_eq!(canonicalize(&json!([])).unwrap(), b"[]");
        assert_eq!(canonicalize(&json!({})).unwrap(), b"{}");
        assert_eq!(canonicalize(&json!(null)).unwrap(), b"null");
    }

    #[test]
    fn strings_escape_minimally() {
        let v = json!("hello\nworld\t\"yes\"");
        assert_eq!(
            canonicalize(&v).unwrap(),
            br#""hello\nworld\t\"yes\"""#
        );
    }

    #[test]
    fn nfc_normalization_applied_to_strings() {
        // Combining sequence "e" + U+0301 must normalize to precomposed "é".
        let decomposed = "e\u{0301}";
        let composed = "\u{00e9}";
        let a = canonicalize(&json!(decomposed)).unwrap();
        let b = canonicalize(&json!(composed)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn same_semantic_value_same_hash() {
        let a = json!({"b": 1, "a": 2});
        let b = json!({"a": 2, "b": 1});
        assert_eq!(predicate_hash(&a).unwrap(), predicate_hash(&b).unwrap());
    }

    #[test]
    fn different_values_different_hash() {
        let a = json!({"a": 1});
        let b = json!({"a": 2});
        assert_ne!(predicate_hash(&a).unwrap(), predicate_hash(&b).unwrap());
    }

    #[test]
    fn sha256_matches_known_vector_empty() {
        // Known: sha256("") = e3b0c442 98fc1c14 9afbf4c8 996fb924 27ae41e4 649b934c a495991b 7852b855
        let h = sha256_hex(b"");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_matches_known_vector_abc() {
        // Known: sha256("abc") = ba7816bf 8f01cfea 414140de 5dae2223 b00361a3 96177a9c b410ff61 f20015ad
        let h = sha256_hex(b"abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn predicate_hash_hex_is_64_chars() {
        let h = predicate_hash(&json!({"x": 1})).unwrap();
        assert_eq!(h.hex().len(), 64);
        assert!(h.hex().chars().all(|c| c.is_ascii_hexdigit()));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib canonical`
Expected: 11 tests, all passing. (The two SHA-256 known-vectors are the bulletproof check that the hand-rolled implementation is correct — if they fail, everything downstream — hashing, caching, dedup — is broken.)

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/canonical.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: canonical JSON + SHA-256 + PredicateHash

Implements Decision 1.1: lexicographic key sort, ryu numbers, NFC strings,
no whitespace. Pure-Rust SHA-256 with two known-vector tests guarding
correctness."
```

---

### Task 1.14: Property test — canonical JSON is stable under key reordering

**Files:**
- Modify: `crates/vcli-core/src/canonical.rs` (append `proptest!` block in the `tests` module)

- [ ] **Step 1: Add a proptest to `tests` in `canonical.rs`**

Append inside the `#[cfg(test)] mod tests` block (before the closing brace):

```rust
use proptest::prelude::*;

fn arb_json_value(depth: u32) -> BoxedStrategy<Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| Value::Number(n.into())),
        "[a-zA-Z0-9 ]{0,20}".prop_map(Value::String),
    ];
    if depth == 0 {
        leaf.boxed()
    } else {
        let inner = arb_json_value(depth - 1);
        prop_oneof![
            leaf,
            proptest::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            proptest::collection::hash_map("[a-z]{1,6}", inner, 0..5)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
        .boxed()
    }
}

proptest! {
    #[test]
    fn canonical_invariant_under_key_shuffle(v in arb_json_value(3)) {
        // Round-trip v through a new Value by reserializing — serde_json::Map
        // preserves insertion order, so the regenerated Map may have different
        // ordering than the original. canonicalize must erase that.
        let s = serde_json::to_string(&v).unwrap();
        let v2: Value = serde_json::from_str(&s).unwrap();
        prop_assert_eq!(canonicalize(&v).unwrap(), canonicalize(&v2).unwrap());
    }

    #[test]
    fn hash_is_stable_across_whitespace_variants(v in arb_json_value(2)) {
        let compact = serde_json::to_string(&v).unwrap();
        let pretty = serde_json::to_string_pretty(&v).unwrap();
        let v_compact: Value = serde_json::from_str(&compact).unwrap();
        let v_pretty: Value = serde_json::from_str(&pretty).unwrap();
        prop_assert_eq!(predicate_hash(&v_compact).unwrap(), predicate_hash(&v_pretty).unwrap());
    }
}
```

- [ ] **Step 2: Run proptest**

Run: `cargo test -p vcli-core --lib canonical`
Expected: 13 tests, all passing (proptest default is 256 cases per property).

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-core/src/canonical.rs
git commit -m "vcli-core: proptest — canonical form stable under key order & whitespace"
```

---

### Task 1.15: Event taxonomy

**Files:**
- Create: `crates/vcli-core/src/events.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod events;` + re-export**

Append to `lib.rs`:

```rust
pub mod events;

pub use events::{Event, EventData};
```

- [ ] **Step 2: Write `events.rs`**

```rust
//! Daemon-wide event taxonomy. Matches spec §IPC → Events (v0).
//!
//! Every variant is tagged with the wire type string via `#[serde(rename)]`
//! so the JSON emitted over IPC matches the spec exactly (e.g. "program.state_changed"
//! rather than the Rust snake_case "program_state_changed").

use serde::{Deserialize, Serialize};

use crate::clock::UnixMs;
use crate::ids::ProgramId;
use crate::state::ProgramState;

/// Envelope pushed on streaming IPC channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Wall-clock timestamp when the event was produced.
    pub at: UnixMs,
    /// Typed payload.
    #[serde(flatten)]
    pub data: EventData,
}

/// Event payloads. Tagged on the wire via `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventData {
    /// A program was accepted by the daemon.
    #[serde(rename = "program.submitted")]
    ProgramSubmitted {
        /// Program id.
        program_id: ProgramId,
        /// Program `name` field.
        name: String,
    },
    /// Program transitioned between lifecycle states.
    #[serde(rename = "program.state_changed")]
    ProgramStateChanged {
        /// Program id.
        program_id: ProgramId,
        /// Prior state.
        from: ProgramState,
        /// New state.
        to: ProgramState,
        /// Human-readable reason.
        reason: String,
    },
    /// Program reached `completed`.
    #[serde(rename = "program.completed")]
    ProgramCompleted {
        /// Program id.
        program_id: ProgramId,
        /// Custom emit name from `on_complete.emit`, if set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emit: Option<String>,
    },
    /// Program reached `failed`.
    #[serde(rename = "program.failed")]
    ProgramFailed {
        /// Program id.
        program_id: ProgramId,
        /// Human-readable reason.
        reason: String,
        /// Step path (e.g. "body[2]") where the failure originated.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<String>,
        /// Custom emit name from `on_fail.emit`, if set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emit: Option<String>,
    },
    /// Program was resumed from a previous daemon_restart failure.
    #[serde(rename = "program.resumed")]
    ProgramResumed {
        /// Program id.
        program_id: ProgramId,
        /// Step index execution resumed at (0 = --from-start).
        from_step: u32,
    },
    /// A watch fired.
    #[serde(rename = "watch.fired")]
    WatchFired {
        /// Program id.
        program_id: ProgramId,
        /// Index into the program's `watches` array.
        watch_index: u32,
        /// Predicate name or `"inline"` for anonymous predicates.
        predicate: String,
    },
    /// An input action was dispatched.
    #[serde(rename = "action.dispatched")]
    ActionDispatched {
        /// Program id.
        program_id: ProgramId,
        /// Serialized action step, for tracing.
        step: serde_json::Value,
        /// Resolved target point if applicable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<crate::geom::Point>,
    },
    /// An action was dropped due to arbiter conflict.
    #[serde(rename = "action.deferred")]
    ActionDeferred {
        /// Program id.
        program_id: ProgramId,
        /// Serialized action step.
        step: serde_json::Value,
        /// Reason for deferral (e.g. `"conflict_with": "<program_id>"`).
        reason: serde_json::Value,
    },
    /// A tick was skipped (e.g. capture overrun).
    #[serde(rename = "tick.frame_skipped")]
    TickFrameSkipped {
        /// Reason tag.
        reason: String,
    },
    /// Scheduler is running sustained over budget (Decision 4.1).
    #[serde(rename = "daemon.pressure")]
    DaemonPressure {
        /// Target tick budget in ms (usually 90).
        tick_budget_ms: u32,
    },
    /// Stream buffer overflow — clients missed events (Decision 1.7).
    #[serde(rename = "stream.dropped")]
    StreamDropped {
        /// Number of dropped events.
        count: u32,
        /// Timestamp of first dropped event.
        since: UnixMs,
    },
    /// Daemon is missing a required permission.
    #[serde(rename = "capture.permission_missing")]
    CapturePermissionMissing {
        /// Backend identifier (e.g. `"screencapturekit"`).
        backend: String,
    },
    /// Daemon started.
    #[serde(rename = "daemon.started")]
    DaemonStarted {
        /// Daemon version string.
        version: String,
    },
    /// Daemon stopped.
    #[serde(rename = "daemon.stopped")]
    DaemonStopped,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_program_id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn program_submitted_serializes_with_typed_tag() {
        let e = Event {
            at: 1_700_000_000_000,
            data: EventData::ProgramSubmitted {
                program_id: sample_program_id(),
                name: "yt".into(),
            },
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains(r#""type":"program.submitted""#), "got {j}");
        assert!(j.contains(r#""name":"yt""#));
        let back: Event = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn program_completed_omits_emit_when_none() {
        let e = Event {
            at: 0,
            data: EventData::ProgramCompleted { program_id: sample_program_id(), emit: None },
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(!j.contains("\"emit\""), "got {j}");
    }

    #[test]
    fn program_failed_carries_optional_step_and_emit() {
        let e = Event {
            at: 0,
            data: EventData::ProgramFailed {
                program_id: sample_program_id(),
                reason: "wait_for timed out".into(),
                step: Some("body[2]".into()),
                emit: Some("buy_failed".into()),
            },
        };
        let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn watch_fired_roundtrip() {
        let e = Event {
            at: 1,
            data: EventData::WatchFired {
                program_id: sample_program_id(),
                watch_index: 0,
                predicate: "skip_visible".into(),
            },
        };
        let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn stream_dropped_roundtrip() {
        let e = Event {
            at: 2,
            data: EventData::StreamDropped { count: 5, since: 100 },
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains(r#""type":"stream.dropped""#));
        let back: Event = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn daemon_pressure_and_frame_skipped_roundtrip() {
        for e in [
            Event { at: 0, data: EventData::DaemonPressure { tick_budget_ms: 90 } },
            Event { at: 0, data: EventData::TickFrameSkipped { reason: "capture_overrun".into() } },
        ] {
            let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
            assert_eq!(back, e);
        }
    }

    #[test]
    fn daemon_started_stopped_roundtrip() {
        for e in [
            Event { at: 1, data: EventData::DaemonStarted { version: "0.0.1".into() } },
            Event { at: 2, data: EventData::DaemonStopped },
        ] {
            let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
            assert_eq!(back, e);
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib events`
Expected: 7 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/events.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: Event taxonomy (13 kinds) with typed serde tags"
```

---

### Task 1.16: Error taxonomy

**Files:**
- Create: `crates/vcli-core/src/error.rs`
- Modify: `crates/vcli-core/src/lib.rs`

- [ ] **Step 1: Add `pub mod error;` + re-export**

Append to `lib.rs`:

```rust
pub mod error;

pub use error::{ErrorCode, ErrorPayload};
```

- [ ] **Step 2: Write `error.rs`**

```rust
//! Error codes and wire payloads shared across the daemon, IPC, and CLI.
//!
//! `ErrorCode` is a stable enum — code strings are part of the IPC contract.
//! `ErrorPayload` mirrors the `{code, message, path?, line?, column?, span_len?}`
//! shape required by Decision 2.2.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable machine-readable error code. String form matches spec §IPC → Error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// DSL validation failed. Accompanied by JSON path.
    InvalidProgram,
    /// Program id not found.
    UnknownProgram,
    /// Illegal state transition, e.g. `cancel` on a completed program.
    BadStateTransition,
    /// macOS Accessibility or Screen Recording permission not granted.
    PermissionDenied,
    /// Capture backend error.
    CaptureFailed,
    /// Daemon is too busy / queue full.
    DaemonBusy,
    /// Resume rejected because program state disqualifies it (Decisions 2.4, C).
    NotResumable,
    /// `vcli resume`: the step N-1 postcondition no longer holds.
    ResumePreconditionFailed,
    /// Catch-all, logged server-side with correlation id.
    Internal,
}

impl ErrorCode {
    /// Wire string form (same as serde rename).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidProgram => "invalid_program",
            Self::UnknownProgram => "unknown_program",
            Self::BadStateTransition => "bad_state_transition",
            Self::PermissionDenied => "permission_denied",
            Self::CaptureFailed => "capture_failed",
            Self::DaemonBusy => "daemon_busy",
            Self::NotResumable => "not_resumable",
            Self::ResumePreconditionFailed => "resume_precondition_failed",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Wire-shape error returned on IPC responses. Decision 2.2 adds line/column/span for
/// parse/validation errors; other codes leave those `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    /// Stable code.
    pub code: ErrorCode,
    /// Human-readable message.
    pub message: String,
    /// JSON path into the offending program (e.g. `watches[0].when`), if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 1-based source line (for DSL parse errors).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based source column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Length of the offending span in characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_len: Option<u32>,
    /// Optional did-you-mean hint (Levenshtein-1 suggestion) — Decision 2.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ErrorPayload {
    /// Minimal constructor for non-DSL errors.
    #[must_use]
    pub fn simple(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
            line: None,
            column: None,
            span_len: None,
            hint: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_serializes_snake_case() {
        for (c, s) in [
            (ErrorCode::InvalidProgram, "invalid_program"),
            (ErrorCode::UnknownProgram, "unknown_program"),
            (ErrorCode::BadStateTransition, "bad_state_transition"),
            (ErrorCode::PermissionDenied, "permission_denied"),
            (ErrorCode::CaptureFailed, "capture_failed"),
            (ErrorCode::DaemonBusy, "daemon_busy"),
            (ErrorCode::NotResumable, "not_resumable"),
            (ErrorCode::ResumePreconditionFailed, "resume_precondition_failed"),
            (ErrorCode::Internal, "internal"),
        ] {
            assert_eq!(c.as_str(), s);
            let j = serde_json::to_string(&c).unwrap();
            assert_eq!(j, format!(r#""{s}""#));
            let back: ErrorCode = serde_json::from_str(&j).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn simple_payload_omits_optional_fields() {
        let p = ErrorPayload::simple(ErrorCode::UnknownProgram, "not found");
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""code":"unknown_program""#));
        assert!(!j.contains("path"));
        assert!(!j.contains("line"));
        assert!(!j.contains("column"));
        assert!(!j.contains("span_len"));
        assert!(!j.contains("hint"));
    }

    #[test]
    fn full_payload_roundtrip() {
        let p = ErrorPayload {
            code: ErrorCode::InvalidProgram,
            message: "unknown predicate 'skp_visible'".into(),
            path: Some("watches[0].when".into()),
            line: Some(12),
            column: Some(18),
            span_len: Some(12),
            hint: Some("skip_visible".into()),
        };
        let back: ErrorPayload = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p vcli-core --lib error`
Expected: 3 tests, all passing.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-core/src/error.rs crates/vcli-core/src/lib.rs
git commit -m "vcli-core: ErrorCode + ErrorPayload (wire shape for IPC errors)"
```

---

### Task 1.17: Full-crate verification

**Files:** (none new — this is a verification task)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test -p vcli-core`
Expected: all tests pass across every module (rough count: ~75+ tests across 12 files).

- [ ] **Step 2: Run clippy in pedantic mode**

Run: `cargo clippy -p vcli-core --all-targets -- -D warnings`
Expected: no warnings, no errors.

- [ ] **Step 3: Run rustfmt check**

Run: `cargo fmt --all -- --check`
Expected: no diff. If any file differs, run `cargo fmt --all` and commit the formatting fix separately.

- [ ] **Step 4: Verify docs build clean**

Run: `cargo doc -p vcli-core --no-deps`
Expected: builds without warnings (the `#![warn(missing_docs)]` in `lib.rs` enforces this).

- [ ] **Step 5: Verify YT fixture roundtrips via canonical JSON**

Add one final cross-module smoke test. Create `crates/vcli-core/tests/fixture_canonical.rs`:

```rust
//! Smoke test: the YT ad skipper fixture canonicalizes and hashes stably.

use vcli_core::canonicalize;
use vcli_core::predicate_hash;
use vcli_core::Program;

const YT_FIXTURE: &str = include_str!("../../../fixtures/yt_ad_skipper.json");

#[test]
fn fixture_canonical_bytes_stable_across_parse_reserialize() {
    let p: Program = serde_json::from_str(YT_FIXTURE).unwrap();
    let reser = serde_json::to_value(&p).unwrap();
    let v1: serde_json::Value = serde_json::from_str(YT_FIXTURE).unwrap();
    assert_eq!(canonicalize(&v1).unwrap(), canonicalize(&reser).unwrap());
}

#[test]
fn fixture_hash_is_stable() {
    let v1: serde_json::Value = serde_json::from_str(YT_FIXTURE).unwrap();
    let v2: serde_json::Value = serde_json::from_str(YT_FIXTURE).unwrap();
    assert_eq!(predicate_hash(&v1).unwrap(), predicate_hash(&v2).unwrap());
}
```

- [ ] **Step 6: Run the smoke test**

Run: `cargo test -p vcli-core --test fixture_canonical`
Expected: 2 tests passing.

- [ ] **Step 7: Final commit**

```bash
git add crates/vcli-core/tests/fixture_canonical.rs
git commit -m "vcli-core: integration smoke test — fixture canonical + hash stable"
```

- [ ] **Step 8: Verify CI would pass locally**

Run in order:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

Expected: all three exit 0 with no output from fmt, no warnings from clippy, all tests pass.

- [ ] **Step 9: Tag the milestone**

```bash
git tag phase-1-vcli-core-complete -m "vcli-core complete — downstream crates unblocked"
```

---

## What this plan unlocks

Once this plan completes, the following parallel lanes can start in separate worktrees — each depends only on `vcli-core`:

- **Lane B: `vcli-dsl`** — parse + validate JSON programs, with Levenshtein did-you-mean. Depends on `Program`, `Predicate`, `Step`, `Region`, `Watch`, `Trigger`, `ErrorPayload`.
- **Lane C: `vcli-capture`** — `Capture` trait + `Frame` + mock + macOS ScreenCaptureKit impl. Depends on `Frame`, `Rect`, `Clock`.
- **Lane D: `vcli-input`** — `Input` trait + `InputAction` + mock + macOS impl. Depends on `InputAction`, `Button`, `Modifier`.
- **Lane E: `vcli-ipc`** — framed JSON codec + `Request`/`Response` types referencing `Event` and `ErrorPayload`. Depends on `Event`, `ErrorPayload`, `ProgramId`, `ProgramState`.
- **Lane F: `vcli-store`** — SQLite schema + migrations + `AssetStore` (content-addressed) + `TraceBuffer`. Depends on `Program`, `ProgramId`, `ProgramState`.
- **Lane G: `vcli-perception`** — `PredicateGraph` + evaluators + `PerceptionCache`. Depends on `Predicate`, `Region`, `PredicateResult`, `PredicateHash`, `Frame`, `Clock`.

The `vcli-runtime`, `vcli-daemon`, and `vcli-cli` crates come later and integrate the lanes above. Each lane gets its own plan written against this spec + any decisions that accrue.

---

## Self-review checklist

- Every step shows the actual code an engineer needs; no "TODO" / "TBD" / "implement later".
- Types used in later tasks (`Program`, `PredicateKind`, `InputAction`) match the spelling and field names in the tasks that define them — verified by the `program.rs` test that imports from every module.
- Every `cargo test …` command corresponds to tests actually written in that task.
- Every file path is absolute-from-repo-root; every commit message names what changed.
- CI config matches the workspace: fmt + clippy on ubuntu; test matrix on ubuntu + macOS.
- All Decision references (1.1, 1.5, 1.6, 2.2, 2.4, C, D, F1, F2, 4.1, 4.3) point to the authoritative spec appendix at `docs/superpowers/specs/2026-04-16-vcli-design.md` §"Review decisions — 2026-04-16".
- Scope check: this plan covers only Phase 0 (bootstrap) + Phase 1 (`vcli-core`). Subsequent lanes each get a separate plan.
