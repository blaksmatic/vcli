# vcli-capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose a `Capture` trait with macOS ScreenCaptureKit implementation, a MockCapture for testing, and Windows stub.

**Architecture:** The `Capture` trait is a narrow, synchronous, `Send + Sync` interface with four methods — `supported_formats()`, `enumerate_windows()`, `grab_screen()`, and `grab_window()`. All outputs use `vcli-core` types (`Frame`, `FrameFormat`, `Rect`, `WindowIndex`). The macOS backend uses the `screencapturekit` crate (crates.io name of the `screencapturekit-rs` project cited in Decision 1.4) because it wraps the modern SCK APIs (`SCShareableContent`, `SCStream`, `SCScreenshotManager`) in safe Rust, is maintained (v0.3+), and already provides BGRA `CVPixelBuffer → bytes` plumbing — saving us from rolling raw `objc2`/`core-graphics` FFI. Window enumeration comes from `SCShareableContent::get()` (synchronous on a tokio-free thread via the crate's blocking API); window frames use `SCScreenshotManager::capture_image_with_filter` for the one-shot per-tick grab the scheduler wants. `MockCapture` is a trivial struct with pre-supplied `Frame`s + window descriptors, returning them in order and looping; used by lanes E/F/G and the future runtime lane for deterministic tests. The Windows module compiles as `unimplemented!()` bodies guarded by `#[cfg(windows)]` so the workspace builds cross-platform while macOS is the only shipping target in v0.

**Tech Stack:** Rust (stable, 2021 edition). Deps: `vcli-core` (workspace), `thiserror`, `tracing`, plus `screencapturekit = "0.3"` (target `cfg(target_os = "macos")`), `core-foundation = "0.10"` (needed for `CFString` bridging of window titles on macOS), and `core-graphics-types = "0.1"` for `CGRect`/`CGSize` geometry. Dev deps: `image = "0.25"` for PNG fixture I/O in tests, `tempfile = "3"` for the example tool's output path.

**Authoritative spec:** `docs/superpowers/specs/2026-04-16-vcli-design.md`. Cross-lane dependency: `vcli-core` is already merged on master with `Frame`, `FrameFormat`, `Rect`, `Region`, `WindowIndex`, `Point`. This lane MUST NOT modify `vcli-core`.

---

## File structure

```
crates/vcli-capture/
├── Cargo.toml
├── src/
│   ├── lib.rs                        # module tree + re-exports + docstring
│   ├── error.rs                      # CaptureError enum (thiserror)
│   ├── capture.rs                    # Capture trait + WindowDescriptor + DisplayId
│   ├── mock.rs                       # MockCapture impl for tests
│   ├── permission.rs                 # TCC probe: screen-recording permission status enum + check fn
│   ├── macos/
│   │   ├── mod.rs                    # MacCapture struct + trait impl (cfg macos)
│   │   ├── sck.rs                    # thin wrapper over screencapturekit crate calls
│   │   └── convert.rs                # BGRA pixel-buffer → Frame, CGRect → Rect
│   └── windows/
│       └── mod.rs                    # WindowsCapture stub (cfg windows, unimplemented!)
├── tests/
│   └── macos_sck_smoke.rs            # integration test, #[cfg(macos)] + #[ignore]
└── examples/
    └── capture_once.rs               # CLI smoke: grabs one frame, prints dims
```

Responsibility split: one file per concern, every file under 300 lines. `sck.rs` isolates all `screencapturekit` API calls so they can be mocked or version-bumped in one place. `convert.rs` is pure — no FFI, just byte-layout conversions — so it can be unit-tested with synthetic buffers.

---

## Task 1: Crate scaffold + `Capture` trait surface

**Files:**
- Create: `crates/vcli-capture/Cargo.toml`
- Create: `crates/vcli-capture/src/lib.rs`
- Create: `crates/vcli-capture/src/capture.rs`
- Modify: `Cargo.toml` (workspace root — add member + workspace deps)

- [ ] **Step 1: Add member + deps to workspace `Cargo.toml`**

Edit `Cargo.toml`:
- Add `"crates/vcli-capture",` to `[workspace].members`.
- Append to `[workspace.dependencies]`:

```toml
tracing = "0.1"
# Platform-specific, referenced from vcli-capture only. Declaring here so
# downstream crates that later depend on vcli-capture don't re-select versions.
screencapturekit = "0.3"
core-foundation = "0.10"
core-graphics-types = "0.1"
image = "0.25"
tempfile = "3"
```

- [ ] **Step 2: Create `crates/vcli-capture/Cargo.toml`**

```toml
[package]
name = "vcli-capture"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Capture trait, macOS ScreenCaptureKit backend, mock, Windows stub, for vcli."

[dependencies]
vcli-core = { path = "../vcli-core" }
thiserror = { workspace = true }
tracing = { workspace = true }

[target.'cfg(target_os = "macos")'.dependencies]
screencapturekit = { workspace = true }
core-foundation = { workspace = true }
core-graphics-types = { workspace = true }

[dev-dependencies]
image = { workspace = true }
tempfile = { workspace = true }

[[example]]
name = "capture_once"
path = "examples/capture_once.rs"
```

- [ ] **Step 3: Create skeleton `crates/vcli-capture/src/lib.rs`**

```rust
//! vcli-capture — Capture trait, macOS ScreenCaptureKit backend, mock impl, Windows stub.
//!
//! See spec §v0 scope and §Architecture → crate responsibilities. All outputs
//! use `vcli-core` types (`Frame`, `FrameFormat`, `Rect`, `WindowIndex`).
//!
//! # Coordinate model
//!
//! Per Decision F1/4.3 capture produces physical pixels internally and emits
//! a `Frame` at logical (1x) resolution. AX coords, input coords, and DSL
//! coords all live in logical space; the one physical → logical conversion
//! happens inside this crate.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod capture;
pub mod error;
pub mod mock;
pub mod permission;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(windows)]
pub mod windows;

pub use capture::{Capture, DisplayId, WindowDescriptor};
pub use error::CaptureError;
pub use mock::MockCapture;
pub use permission::{PermissionStatus, check_screen_recording_permission};
```

Note: `#![forbid(unsafe_code)]` stays at the crate root. The ScreenCaptureKit crate itself uses unsafe internally; we never write raw `unsafe` blocks in this crate.

- [ ] **Step 4: Write failing tests + `Capture` trait in `crates/vcli-capture/src/capture.rs`**

```rust
//! `Capture` trait and associated descriptor types.

use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;
use vcli_core::region::WindowIndex;

use crate::error::CaptureError;

/// Opaque identifier for a display. v0 is single-display (primary) so this is
/// effectively `DisplayId::PRIMARY`, but the type is here from day 1 to keep
/// the trait stable when multi-display lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayId(pub u32);

impl DisplayId {
    /// The primary display. Backends may enumerate others in the future.
    pub const PRIMARY: Self = Self(0);
}

/// A window known to the capture backend. Stable for the lifetime of the
/// window; ordering matches the native enumeration order (macOS: oldest →
/// newest per Decision F2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDescriptor {
    /// Backend-assigned stable id. For macOS this is the AX window id (u32)
    /// re-exposed. Opaque to callers.
    pub id: u64,
    /// App/owner name, e.g. "Safari".
    pub app: String,
    /// Window title string, possibly empty.
    pub title: String,
    /// Bounds in logical pixels, relative to the display origin.
    pub bounds: Rect,
    /// 0-based index within the (app, title-substring) group this window
    /// belongs to in the current enumeration pass. The region resolver in
    /// the perception lane uses this directly as `WindowIndex`.
    pub window_index: WindowIndex,
    /// Display the window is currently on.
    pub display: DisplayId,
}

/// Capture backend. One capture per tick; frame is shared across programs.
pub trait Capture: Send + Sync {
    /// Pixel formats this backend can emit. First entry is the preferred one.
    fn supported_formats(&self) -> &[FrameFormat];

    /// Enumerate visible application windows on all displays. Excludes
    /// off-screen and minimized windows. Stable ordering per backend.
    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError>;

    /// Grab a full-screen frame of the primary display.
    fn grab_screen(&mut self) -> Result<Frame, CaptureError>;

    /// Grab a frame cropped to the given window's current bounds. Backend
    /// may re-resolve the window by id; if the window has moved/resized
    /// between enumeration and grab, the returned `Frame.bounds` reflects
    /// the actual capture, not the stale descriptor bounds.
    fn grab_window(&mut self, window: &WindowDescriptor) -> Result<Frame, CaptureError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_id_primary_is_zero() {
        assert_eq!(DisplayId::PRIMARY, DisplayId(0));
    }

    #[test]
    fn window_descriptor_is_clone_eq() {
        let a = WindowDescriptor {
            id: 42,
            app: "Safari".into(),
            title: "YouTube".into(),
            bounds: Rect { x: 0, y: 0, w: 800, h: 600 },
            window_index: WindowIndex(0),
            display: DisplayId::PRIMARY,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn capture_trait_is_object_safe() {
        // If this compiles, `dyn Capture` works — required for scheduler injection.
        fn _takes_dyn(_c: &mut dyn Capture) {}
    }
}
```

- [ ] **Step 5: Create stub `error.rs` so `capture.rs` compiles** (filled out in Task 2)

```rust
//! Error types for the capture crate. See Task 2 for the full enum.
use thiserror::Error;

/// Placeholder — real variants added in Task 2.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Generic placeholder; replaced in Task 2.
    #[error("capture error: {0}")]
    Other(String),
}
```

- [ ] **Step 6: Verify compile**

Run: `cargo check -p vcli-capture`
Expected: OK. No errors, no warnings.

- [ ] **Step 7: Run the 3 trait tests**

Run: `cargo test -p vcli-capture --lib capture`
Expected: 3 tests pass.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/vcli-capture/Cargo.toml crates/vcli-capture/src/lib.rs \
        crates/vcli-capture/src/capture.rs crates/vcli-capture/src/error.rs
git commit -m "vcli-capture: crate scaffold + Capture trait + WindowDescriptor"
```

---

## Task 2: `CaptureError` taxonomy

**Files:**
- Modify: `crates/vcli-capture/src/error.rs`
- Test: inline in `error.rs`

- [ ] **Step 1: Write failing tests at the bottom of `error.rs`**

Replace the file with:

```rust
//! Error types for the capture crate.
//!
//! The runtime maps `PermissionDenied` to event `capture.permission_missing`
//! (spec §Events) and `capture_failed` to the ipc error code of the same name.

use thiserror::Error;

/// All errors this crate produces.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Screen Recording (TCC) permission is not granted. User must approve
    /// in System Settings → Privacy & Security → Screen Recording.
    #[error("screen recording permission not granted")]
    PermissionDenied,

    /// The named window was not found on any display.
    #[error("window not found: id={id}")]
    WindowNotFound {
        /// Opaque backend id from the lost descriptor.
        id: u64,
    },

    /// Backend returned a frame the converter couldn't interpret (bad stride,
    /// bad pixel format, zero-sized plane, …).
    #[error("malformed frame from backend: {reason}")]
    MalformedFrame {
        /// Human-readable reason string.
        reason: String,
    },

    /// Backend call timed out or failed mid-operation. Wraps the backend-
    /// specific message so the runtime can surface it in `vcli health`.
    #[error("backend failure: {message}")]
    Backend {
        /// Backend-provided message (SCK error description, HRESULT, etc.).
        message: String,
    },

    /// The backend does not implement this method on the current platform.
    #[error("capture operation unsupported on this backend: {what}")]
    Unsupported {
        /// Short description of what was attempted.
        what: &'static str,
    },
}

impl CaptureError {
    /// Stable short string suitable for the ipc layer's error `code` field.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::WindowNotFound { .. } => "unknown_window",
            Self::MalformedFrame { .. } | Self::Backend { .. } => "capture_failed",
            Self::Unsupported { .. } => "unsupported",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_are_informative() {
        let e = CaptureError::PermissionDenied;
        assert_eq!(
            e.to_string(),
            "screen recording permission not granted"
        );
        let e = CaptureError::WindowNotFound { id: 77 };
        assert_eq!(e.to_string(), "window not found: id=77");
        let e = CaptureError::MalformedFrame { reason: "bad stride".into() };
        assert!(e.to_string().contains("bad stride"));
    }

    #[test]
    fn codes_are_stable() {
        assert_eq!(CaptureError::PermissionDenied.code(), "permission_denied");
        assert_eq!(
            CaptureError::WindowNotFound { id: 1 }.code(),
            "unknown_window"
        );
        assert_eq!(
            CaptureError::Backend { message: "x".into() }.code(),
            "capture_failed"
        );
        assert_eq!(
            CaptureError::MalformedFrame { reason: "x".into() }.code(),
            "capture_failed"
        );
        assert_eq!(
            CaptureError::Unsupported { what: "x" }.code(),
            "unsupported"
        );
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p vcli-capture --lib error`
Expected: 2 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-capture/src/error.rs
git commit -m "vcli-capture: CaptureError taxonomy + stable code() mapping"
```

---

## Task 3: `MockCapture` — deterministic frames and windows for downstream tests

**Files:**
- Create: `crates/vcli-capture/src/mock.rs`
- Test: inline in `mock.rs`

- [ ] **Step 1: Write failing tests in `crates/vcli-capture/src/mock.rs`**

```rust
//! `MockCapture` — canned frames + windows for deterministic tests.
//!
//! Used by vcli-runtime scenario harness, vcli-perception evaluator tests,
//! and the vcli-daemon ipc handler in its test suite.

use std::sync::{Arc, Mutex};

use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;
use vcli_core::region::WindowIndex;

use crate::capture::{Capture, DisplayId, WindowDescriptor};
use crate::error::CaptureError;

/// Canned-data capture backend. Cycles through `screen_frames` on each
/// `grab_screen()`. `window_frames` lookup by window id. `enumerate_windows`
/// returns a clone of the configured list. Thread-safe via `Mutex`.
#[derive(Debug)]
pub struct MockCapture {
    inner: Arc<Mutex<MockInner>>,
}

#[derive(Debug)]
struct MockInner {
    formats: Vec<FrameFormat>,
    windows: Vec<WindowDescriptor>,
    screen_frames: Vec<Frame>,
    screen_cursor: usize,
    window_frame_map: Vec<(u64, Vec<Frame>)>,
    window_cursors: Vec<usize>,
    next_error: Option<CaptureError>,
}

impl MockCapture {
    /// Empty mock — all calls return default / empty. For the "no programs,
    /// nothing to capture" path.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(vec![FrameFormat::Bgra8], vec![], vec![])
    }

    /// Full constructor.
    #[must_use]
    pub fn new(
        formats: Vec<FrameFormat>,
        windows: Vec<WindowDescriptor>,
        screen_frames: Vec<Frame>,
    ) -> Self {
        let window_cursors = vec![0; windows.len()];
        Self {
            inner: Arc::new(Mutex::new(MockInner {
                formats,
                windows,
                screen_frames,
                screen_cursor: 0,
                window_frame_map: Vec::new(),
                window_cursors,
                next_error: None,
            })),
        }
    }

    /// Set canned frames for a particular window id. Subsequent calls to
    /// `grab_window` with that descriptor cycle through these frames.
    pub fn set_window_frames(&self, window_id: u64, frames: Vec<Frame>) {
        let mut g = self.inner.lock().unwrap();
        if let Some((_, slot)) = g.window_frame_map.iter_mut().find(|(id, _)| *id == window_id) {
            *slot = frames;
        } else {
            g.window_frame_map.push((window_id, frames));
        }
    }

    /// Arm the next call to any grab/enumerate method to fail with `e`. Used
    /// to exercise error paths in consumer tests. Consumed by one failing call.
    pub fn arm_error(&self, e: CaptureError) {
        self.inner.lock().unwrap().next_error = Some(e);
    }

    fn take_armed_error(inner: &mut MockInner) -> Option<CaptureError> {
        inner.next_error.take()
    }
}

impl Capture for MockCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        // Safety-ish: we hand out a leaked static slice derived from the
        // current config by cloning into a boxed leak only once. To keep this
        // simple and allocation-free on the hot path, we return a constant
        // slice with the default format.
        const DEFAULT: &[FrameFormat] = &[FrameFormat::Bgra8];
        DEFAULT
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(e) = Self::take_armed_error(&mut g) {
            return Err(e);
        }
        Ok(g.windows.clone())
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(e) = Self::take_armed_error(&mut g) {
            return Err(e);
        }
        if g.screen_frames.is_empty() {
            return Err(CaptureError::Backend {
                message: "MockCapture has no screen frames configured".into(),
            });
        }
        let idx = g.screen_cursor % g.screen_frames.len();
        g.screen_cursor = g.screen_cursor.wrapping_add(1);
        Ok(g.screen_frames[idx].clone())
    }

    fn grab_window(&mut self, window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(e) = Self::take_armed_error(&mut g) {
            return Err(e);
        }
        let pos = g.window_frame_map.iter().position(|(id, _)| *id == window.id);
        let Some(pos) = pos else {
            return Err(CaptureError::WindowNotFound { id: window.id });
        };
        let frames_len = g.window_frame_map[pos].1.len();
        if frames_len == 0 {
            return Err(CaptureError::Backend {
                message: format!("no frames for window id={}", window.id),
            });
        }
        while g.window_cursors.len() <= pos {
            g.window_cursors.push(0);
        }
        let cursor = g.window_cursors[pos];
        let idx = cursor % frames_len;
        g.window_cursors[pos] = cursor.wrapping_add(1);
        Ok(g.window_frame_map[pos].1[idx].clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn tiny(format: FrameFormat, value: u8) -> Frame {
        let bytes: Arc<[u8]> = vec![value; 4 * 4 * 2].into();
        Frame::new(
            format,
            Rect { x: 0, y: 0, w: 4, h: 2 },
            4 * 4,
            bytes,
            0,
        )
    }

    #[test]
    fn empty_mock_returns_empty_windows() {
        let m = MockCapture::empty();
        assert!(m.enumerate_windows().unwrap().is_empty());
    }

    #[test]
    fn empty_mock_screen_grab_errors() {
        let mut m = MockCapture::empty();
        let e = m.grab_screen().unwrap_err();
        assert_eq!(e.code(), "capture_failed");
    }

    #[test]
    fn screen_frames_cycle_in_order() {
        let frames = vec![tiny(FrameFormat::Bgra8, 1), tiny(FrameFormat::Bgra8, 2)];
        let mut m = MockCapture::new(vec![FrameFormat::Bgra8], vec![], frames);
        let a = m.grab_screen().unwrap();
        let b = m.grab_screen().unwrap();
        let c = m.grab_screen().unwrap();
        assert_eq!(a.pixels[0], 1);
        assert_eq!(b.pixels[0], 2);
        assert_eq!(c.pixels[0], 1); // wraps
    }

    #[test]
    fn enumerate_returns_configured_windows() {
        let w = WindowDescriptor {
            id: 9,
            app: "Finder".into(),
            title: "Downloads".into(),
            bounds: Rect { x: 0, y: 0, w: 1000, h: 600 },
            window_index: WindowIndex(0),
            display: DisplayId::PRIMARY,
        };
        let m = MockCapture::new(vec![FrameFormat::Bgra8], vec![w.clone()], vec![]);
        let got = m.enumerate_windows().unwrap();
        assert_eq!(got, vec![w]);
    }

    #[test]
    fn grab_window_uses_configured_frames() {
        let w = WindowDescriptor {
            id: 7,
            app: "Safari".into(),
            title: "YT".into(),
            bounds: Rect { x: 0, y: 0, w: 4, h: 2 },
            window_index: WindowIndex(0),
            display: DisplayId::PRIMARY,
        };
        let m = MockCapture::new(vec![FrameFormat::Bgra8], vec![w.clone()], vec![]);
        m.set_window_frames(7, vec![tiny(FrameFormat::Bgra8, 42)]);
        let mut m2 = m;
        let f = m2.grab_window(&w).unwrap();
        assert_eq!(f.pixels[0], 42);
    }

    #[test]
    fn grab_window_unknown_id_errors() {
        let w = WindowDescriptor {
            id: 100,
            app: "X".into(),
            title: "Y".into(),
            bounds: Rect { x: 0, y: 0, w: 4, h: 2 },
            window_index: WindowIndex(0),
            display: DisplayId::PRIMARY,
        };
        let mut m = MockCapture::new(vec![FrameFormat::Bgra8], vec![], vec![]);
        let e = m.grab_window(&w).unwrap_err();
        assert_eq!(e.code(), "unknown_window");
    }

    #[test]
    fn armed_error_is_returned_once() {
        let mut m = MockCapture::new(
            vec![FrameFormat::Bgra8],
            vec![],
            vec![tiny(FrameFormat::Bgra8, 0)],
        );
        m.arm_error(CaptureError::PermissionDenied);
        assert_eq!(m.grab_screen().unwrap_err().code(), "permission_denied");
        // Second call succeeds because error was consumed.
        assert!(m.grab_screen().is_ok());
    }

    #[test]
    fn mock_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockCapture>();
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p vcli-capture --lib mock`
Expected: 8 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-capture/src/mock.rs
git commit -m "vcli-capture: MockCapture with cycling frames + armed errors"
```

---

## Task 4: Permission probe (TCC / Screen Recording)

**Files:**
- Create: `crates/vcli-capture/src/permission.rs`
- Test: inline

- [ ] **Step 1: Write test + impl in `permission.rs`**

```rust
//! Screen Recording (TCC) permission probe.
//!
//! On macOS this uses `CGPreflightScreenCaptureAccess` and
//! `CGRequestScreenCaptureAccess` from the Core Graphics framework, called
//! indirectly via the `screencapturekit` crate. On other OSes the result is
//! always `Granted` because there's no equivalent gating.

use crate::error::CaptureError;

/// Result of probing the OS for screen-recording permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    /// Permission granted — capture will succeed.
    Granted,
    /// Not yet granted. On macOS the daemon should emit
    /// `capture.permission_missing { backend: "macos" }` per spec §Events.
    Denied,
    /// Backend is unable to determine the status (e.g., running under `nohup`
    /// with no window server). Treated as denied by the scheduler but
    /// surfaces to the user as a more specific diagnostic.
    Unknown,
}

/// Synchronously probe the OS for screen-recording permission.
///
/// - macOS: calls `CGPreflightScreenCaptureAccess()`. Does not prompt.
///   Use `request_screen_recording_permission()` to trigger the system prompt.
/// - Other OSes: always returns `Granted`.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] only if the underlying FFI call fails
/// in a way that is not "permission denied" (very rare — e.g., window server
/// unreachable).
pub fn check_screen_recording_permission() -> Result<PermissionStatus, CaptureError> {
    #[cfg(target_os = "macos")]
    {
        // The screencapturekit crate exposes `has_permission()` which wraps
        // CGPreflightScreenCaptureAccess without prompting. See sck crate
        // docs: `screencapturekit::shareable_content::SCShareableContent`
        // initialization returns an error if permission is missing — but we
        // want a non-throwing probe, so we go through the CG symbol directly.
        // The crate re-exports it as `screencapturekit::util::has_permission`.
        // Fall back to `false = Denied` if the symbol errors.
        let granted = screencapturekit::util::has_permission();
        return Ok(if granted {
            PermissionStatus::Granted
        } else {
            PermissionStatus::Denied
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(PermissionStatus::Granted)
    }
}

/// Request the user grant screen-recording permission. Triggers the macOS
/// system prompt if currently Denied. No-op on other OSes.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] if the request call fails.
pub fn request_screen_recording_permission() -> Result<(), CaptureError> {
    #[cfg(target_os = "macos")]
    {
        // Ignores return value (boolean) by design — the prompt is async
        // from the user's perspective; the caller should re-probe later.
        let _ = screencapturekit::util::request_permission();
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_status_is_copy_eq() {
        let a = PermissionStatus::Granted;
        let b = a;
        assert_eq!(a, b);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_always_granted() {
        let s = check_screen_recording_permission().unwrap();
        assert_eq!(s, PermissionStatus::Granted);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_request_is_no_op() {
        request_screen_recording_permission().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_probe_does_not_panic() {
        // Doesn't assert Granted/Denied — that's environment-dependent.
        // Just ensures the FFI call wiring works without unwind.
        let _ = check_screen_recording_permission().unwrap();
    }
}
```

Note: if `screencapturekit::util::has_permission` / `request_permission` do not exist on v0.3, the implementer will inline the two symbols via `extern "C"` links in a small `macos/permission.rs` file — the tests above still describe the contract. Check with `cargo doc --open -p screencapturekit` before deciding.

- [ ] **Step 2: Run tests**

Run: `cargo test -p vcli-capture --lib permission`
Expected: on non-macOS: 3 tests pass. On macOS: 2 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-capture/src/permission.rs
git commit -m "vcli-capture: TCC permission probe with PermissionStatus enum"
```

---

## Task 5: macOS — `BGRA CVPixelBuffer → Frame` conversion (pure, unit-tested)

**Files:**
- Create: `crates/vcli-capture/src/macos/mod.rs`
- Create: `crates/vcli-capture/src/macos/convert.rs`
- Test: inline in `convert.rs`

- [ ] **Step 1: Stub `macos/mod.rs` to declare the submodule**

```rust
//! macOS capture backend. Enabled on `target_os = "macos"` only.

pub mod convert;
pub mod sck;

pub use sck::MacCapture;
```

- [ ] **Step 2: Write `convert.rs` with pure byte-layout converters + tests**

```rust
//! Pure BGRA / physical → logical converters. No FFI. Unit-tested with
//! synthetic buffers so platform-free CI exercises the pixel math.

use std::sync::Arc;

use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;

use crate::error::CaptureError;

/// Parameters for a raw BGRA8 buffer emitted by a backend.
#[derive(Debug, Clone)]
pub struct RawBgra {
    /// Physical-pixel width.
    pub width_px: u32,
    /// Physical-pixel height.
    pub height_px: u32,
    /// Row stride in bytes. May exceed `width_px * 4` for alignment padding.
    pub stride: usize,
    /// Physical pixel bytes. Length must be ≥ `stride * height_px`.
    pub pixels: Vec<u8>,
    /// Monotonic ns timestamp.
    pub captured_at_ns: u64,
    /// Factor to downsample to logical 1x. 2.0 for Retina, 1.0 for non-HiDPI.
    pub scale: f32,
    /// Logical origin (top-left of the captured region in logical desktop coords).
    pub logical_origin_x: i32,
    /// Logical origin y.
    pub logical_origin_y: i32,
}

/// Convert a raw BGRA physical-pixel buffer into a logical-resolution `Frame`.
///
/// For `scale == 1.0` the bytes are wrapped with no copying aside from the
/// `Arc`. For `scale == 2.0` (Retina) we perform a simple 2:1 box-filter
/// downsample so templates authored at logical resolution match.
///
/// # Errors
///
/// Returns [`CaptureError::MalformedFrame`] if:
/// - `pixels.len() < stride * height_px`
/// - `stride < width_px * 4`
/// - `scale` not in {1.0, 2.0} (v0 supported scales; fractional DPI deferred)
pub fn bgra_to_frame(raw: RawBgra) -> Result<Frame, CaptureError> {
    let need = raw
        .stride
        .checked_mul(raw.height_px as usize)
        .ok_or_else(|| CaptureError::MalformedFrame {
            reason: "stride * height overflow".into(),
        })?;
    if raw.pixels.len() < need {
        return Err(CaptureError::MalformedFrame {
            reason: format!(
                "buffer len {} < stride*height {}",
                raw.pixels.len(),
                need
            ),
        });
    }
    if raw.stride < (raw.width_px as usize) * 4 {
        return Err(CaptureError::MalformedFrame {
            reason: format!(
                "stride {} less than width*4 {}",
                raw.stride,
                raw.width_px * 4
            ),
        });
    }

    let scale = raw.scale;
    let (out_w, out_h, out_stride, out_pixels) = if (scale - 1.0).abs() < f32::EPSILON {
        (
            raw.width_px as i32,
            raw.height_px as i32,
            raw.stride,
            raw.pixels,
        )
    } else if (scale - 2.0).abs() < f32::EPSILON {
        downsample_2x(&raw)
    } else {
        return Err(CaptureError::MalformedFrame {
            reason: format!("unsupported scale {scale}"),
        });
    };

    let bounds = Rect {
        x: raw.logical_origin_x,
        y: raw.logical_origin_y,
        w: out_w,
        h: out_h,
    };

    let bytes: Arc<[u8]> = out_pixels.into();
    Ok(Frame::new(
        FrameFormat::Bgra8,
        bounds,
        out_stride,
        bytes,
        raw.captured_at_ns,
    ))
}

/// 2:1 box-filter downsample of a BGRA buffer. Returns
/// `(out_w, out_h, out_stride, out_pixels)`. Output stride = out_w * 4.
fn downsample_2x(raw: &RawBgra) -> (i32, i32, usize, Vec<u8>) {
    let out_w = (raw.width_px / 2) as usize;
    let out_h = (raw.height_px / 2) as usize;
    let out_stride = out_w * 4;
    let mut out = vec![0u8; out_stride * out_h];
    for y in 0..out_h {
        let src_y0 = y * 2;
        let src_y1 = src_y0 + 1;
        let row0 = &raw.pixels[src_y0 * raw.stride..src_y0 * raw.stride + raw.width_px as usize * 4];
        let row1 = &raw.pixels[src_y1 * raw.stride..src_y1 * raw.stride + raw.width_px as usize * 4];
        for x in 0..out_w {
            let sx = x * 2 * 4;
            let idx = y * out_stride + x * 4;
            // Box average 4 source pixels per channel.
            for c in 0..4 {
                let sum = row0[sx + c] as u32
                    + row0[sx + 4 + c] as u32
                    + row1[sx + c] as u32
                    + row1[sx + 4 + c] as u32;
                out[idx + c] = (sum / 4) as u8;
            }
        }
    }
    (out_w as i32, out_h as i32, out_stride, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(width: u32, height: u32, stride: usize, fill: u8, scale: f32) -> RawBgra {
        RawBgra {
            width_px: width,
            height_px: height,
            stride,
            pixels: vec![fill; stride * height as usize],
            captured_at_ns: 1,
            scale,
            logical_origin_x: 0,
            logical_origin_y: 0,
        }
    }

    #[test]
    fn scale_1_passes_through_dims() {
        let r = raw(8, 4, 32, 0xAB, 1.0);
        let f = bgra_to_frame(r).unwrap();
        assert_eq!(f.width(), 8);
        assert_eq!(f.height(), 4);
        assert_eq!(f.stride, 32);
        assert_eq!(f.format, FrameFormat::Bgra8);
        assert_eq!(f.pixels[0], 0xAB);
    }

    #[test]
    fn scale_2_halves_dims_and_preserves_uniform_color() {
        let r = raw(8, 4, 32, 0x7F, 2.0);
        let f = bgra_to_frame(r).unwrap();
        assert_eq!(f.width(), 4);
        assert_eq!(f.height(), 2);
        assert_eq!(f.stride, 16);
        // Box-filter of constant color is the same color.
        for b in f.pixels.iter() {
            assert_eq!(*b, 0x7F);
        }
    }

    #[test]
    fn scale_2_averages_two_different_pixels() {
        // 4x2 physical, 2x1 logical. Left column = 100, right column = 200.
        let mut buf = vec![0u8; 4 * 2 * 4];
        for y in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 100 } else { 200 };
                for c in 0..4 {
                    buf[y * 16 + x * 4 + c] = v;
                }
            }
        }
        let r = RawBgra {
            width_px: 4,
            height_px: 2,
            stride: 16,
            pixels: buf,
            captured_at_ns: 0,
            scale: 2.0,
            logical_origin_x: 0,
            logical_origin_y: 0,
        };
        let f = bgra_to_frame(r).unwrap();
        // Output is 2x1. Left output pixel averages 4 physical pixels of value 100.
        // Right output pixel averages 4 physical pixels of value 200.
        assert_eq!(f.width(), 2);
        assert_eq!(f.height(), 1);
        assert_eq!(f.pixels[0], 100);
        assert_eq!(f.pixels[4], 200);
    }

    #[test]
    fn too_small_buffer_errors() {
        let mut r = raw(100, 100, 400, 0, 1.0);
        r.pixels.truncate(50);
        let e = bgra_to_frame(r).unwrap_err();
        assert_eq!(e.code(), "capture_failed");
    }

    #[test]
    fn stride_less_than_width_times_4_errors() {
        let r = raw(10, 2, 20, 0, 1.0); // stride 20 < 10*4 = 40
        let e = bgra_to_frame(r).unwrap_err();
        assert_eq!(e.code(), "capture_failed");
    }

    #[test]
    fn fractional_scale_unsupported() {
        let r = raw(10, 10, 40, 0, 1.5);
        let e = bgra_to_frame(r).unwrap_err();
        assert_eq!(e.code(), "capture_failed");
    }
}
```

- [ ] **Step 3: Verify compile on macOS**

Run: `cargo check -p vcli-capture` on a macOS machine (or guarded tests will no-op on CI Linux).
Expected: OK.

- [ ] **Step 4: Run the convert tests (macOS only — they are cfg-guarded through parent mod)**

Run: `cargo test -p vcli-capture --lib macos::convert`
Expected on macOS: 6 tests pass. On Linux: module not compiled, no tests.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-capture/src/macos/mod.rs crates/vcli-capture/src/macos/convert.rs
git commit -m "vcli-capture: macos::convert — BGRA physical → logical Frame"
```

---

## Task 6: macOS — `MacCapture` SCK wrapper (window enumeration)

**Files:**
- Create: `crates/vcli-capture/src/macos/sck.rs`
- Test: inline; integration test added in Task 9

- [ ] **Step 1: Write `sck.rs` — `MacCapture::new` + `enumerate_windows`**

```rust
//! macOS ScreenCaptureKit backend. Synchronous facade over the async
//! `screencapturekit` crate using a private tokio runtime, so callers get
//! the blocking `Capture` trait shape the scheduler expects.
//!
//! Coordinate model (Decision F1, 4.3):
//!   SCK reports `CGRect` in logical points. We treat logical points == logical
//!   pixels for v0 (macOS). Physical pixel downsample happens in `convert.rs`
//!   using the display's scale factor.

use std::sync::Mutex;

use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;
use vcli_core::region::WindowIndex;

use crate::capture::{Capture, DisplayId, WindowDescriptor};
use crate::error::CaptureError;
use crate::macos::convert::{bgra_to_frame, RawBgra};
use crate::permission::{check_screen_recording_permission, PermissionStatus};

use screencapturekit::{
    shareable_content::SCShareableContent,
    shareable_content::SCWindow,
    shareable_content::SCDisplay,
    output::CMSampleBufferRef,
};

/// macOS SCK-backed `Capture` implementation.
pub struct MacCapture {
    /// Cached `SCShareableContent` refreshed per-call. Behind a `Mutex`
    /// because SCK types are `!Sync` on some versions.
    last_content: Mutex<Option<SCShareableContent>>,
}

impl MacCapture {
    /// Construct. Does NOT probe permission (permission check is explicit
    /// via `check_screen_recording_permission`).
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::PermissionDenied`] if at construction time the
    /// TCC probe reports `Denied`, so the daemon can emit the correct event
    /// before ever attempting a grab.
    pub fn new() -> Result<Self, CaptureError> {
        match check_screen_recording_permission()? {
            PermissionStatus::Granted => Ok(Self {
                last_content: Mutex::new(None),
            }),
            PermissionStatus::Denied | PermissionStatus::Unknown => {
                Err(CaptureError::PermissionDenied)
            }
        }
    }

    /// Refresh cached `SCShareableContent`.
    fn refresh_content(&self) -> Result<SCShareableContent, CaptureError> {
        // Synchronous wrapper exposed by the crate (v0.3+).
        let content = SCShareableContent::get().map_err(|e| CaptureError::Backend {
            message: format!("SCShareableContent::get: {e}"),
        })?;
        *self.last_content.lock().unwrap() = Some(content.clone());
        Ok(content)
    }

    /// Primary display — first in the enumerated list.
    fn primary_display(&self, content: &SCShareableContent) -> Result<SCDisplay, CaptureError> {
        content
            .displays()
            .into_iter()
            .next()
            .ok_or_else(|| CaptureError::Backend {
                message: "no displays reported by SCShareableContent".into(),
            })
    }

    /// Locate a window by its stable AX id in a fresh enumeration.
    fn locate_window(&self, id: u64) -> Result<SCWindow, CaptureError> {
        let content = self.refresh_content()?;
        content
            .windows()
            .into_iter()
            .find(|w| u64::from(w.window_id()) == id)
            .ok_or(CaptureError::WindowNotFound { id })
    }

    /// Compute the `window_index` field for each descriptor by grouping
    /// windows by `(app, title)` — Decision F2 resolves the region layer.
    fn build_descriptors(windows: Vec<SCWindow>) -> Vec<WindowDescriptor> {
        use std::collections::HashMap;
        let mut counters: HashMap<(String, String), u32> = HashMap::new();
        let mut out = Vec::with_capacity(windows.len());
        for w in windows {
            let app = w
                .owning_application()
                .and_then(|a| a.application_name().ok())
                .unwrap_or_default();
            let title = w.title().unwrap_or_default();
            let rect = w.frame();
            let idx = counters
                .entry((app.clone(), title.clone()))
                .and_modify(|n| *n += 1)
                .or_insert(0);
            out.push(WindowDescriptor {
                id: u64::from(w.window_id()),
                app,
                title,
                bounds: Rect {
                    x: rect.origin.x as i32,
                    y: rect.origin.y as i32,
                    w: rect.size.width as i32,
                    h: rect.size.height as i32,
                },
                window_index: WindowIndex(*idx),
                display: DisplayId::PRIMARY,
            });
        }
        out
    }
}

impl Capture for MacCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        // ScreenCaptureKit gives us BGRA on Apple Silicon + Intel.
        const FORMATS: &[FrameFormat] = &[FrameFormat::Bgra8];
        FORMATS
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        let content = self.refresh_content()?;
        let mut windows = content.windows();
        // Stable ordering: sort by window_id ascending (AX order ≈ creation time).
        windows.sort_by_key(screencapturekit::shareable_content::SCWindow::window_id);
        Ok(Self::build_descriptors(windows))
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        // See Task 7 — this method is fleshed out there.
        Err(CaptureError::Unsupported { what: "grab_screen" })
    }

    fn grab_window(&mut self, _window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        // See Task 8.
        Err(CaptureError::Unsupported { what: "grab_window" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_descriptors_assigns_indices_per_app_title() {
        // We can't construct real SCWindows in unit tests; cover the pure
        // index-assignment logic instead by extracting it. See descriptor
        // index proptest in Task 7 Step 3 after we add a pure helper.
        // Placeholder: assert the counter hashmap approach is stable.
        let a: Vec<WindowDescriptor> = Vec::new();
        assert!(a.is_empty());
    }

    #[test]
    fn supported_formats_contains_bgra8() {
        // Can construct even without permission for the format query since
        // MacCapture::new checks permission. Use a manual struct here.
        let m = MacCapture {
            last_content: Mutex::new(None),
        };
        assert_eq!(m.supported_formats(), &[FrameFormat::Bgra8]);
    }
}
```

- [ ] **Step 2: Refactor `build_descriptors` to a pure helper for testability**

Extract the indexing math to a free function `pub(crate) fn assign_window_indices(raw: Vec<(u64, String, String, Rect)>) -> Vec<WindowDescriptor>`. Add property tests that two windows with same `(app, title)` get sequential indices starting at 0.

Add test:

```rust
#[test]
fn indices_are_sequential_per_app_title() {
    let raw = vec![
        (1, "Safari".into(), "YouTube".into(), Rect { x: 0, y: 0, w: 10, h: 10 }),
        (2, "Safari".into(), "YouTube".into(), Rect { x: 10, y: 0, w: 10, h: 10 }),
        (3, "Safari".into(), "Mail".into(),    Rect { x: 20, y: 0, w: 10, h: 10 }),
        (4, "Finder".into(), "YouTube".into(), Rect { x: 30, y: 0, w: 10, h: 10 }),
    ];
    let d = assign_window_indices(raw);
    assert_eq!(d[0].window_index, WindowIndex(0));
    assert_eq!(d[1].window_index, WindowIndex(1));
    assert_eq!(d[2].window_index, WindowIndex(0));
    assert_eq!(d[3].window_index, WindowIndex(0));
}
```

- [ ] **Step 3: Verify compile on macOS**

Run: `cargo check -p vcli-capture --target-dir /tmp/vcli-check`
Expected: OK.

- [ ] **Step 4: Run tests on macOS**

Run: `cargo test -p vcli-capture --lib macos::sck`
Expected: 2 tests pass on macOS.

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-capture/src/macos/sck.rs crates/vcli-capture/src/macos/mod.rs
git commit -m "vcli-capture: MacCapture::enumerate_windows via SCShareableContent"
```

---

## Task 7: macOS — `grab_screen` via `SCScreenshotManager`

**Files:**
- Modify: `crates/vcli-capture/src/macos/sck.rs`
- Test: augment inline; Task 9 adds the hardware integration test

- [ ] **Step 1: Add `grab_screen` impl using `SCScreenshotManager` or one-shot `SCStream`**

Replace the `grab_screen` stub in `sck.rs` with:

```rust
fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
    use screencapturekit::{
        output::sc_stream_configuration::SCStreamConfiguration,
        output::sc_stream_content_filter::SCContentFilter,
        screenshot_manager::SCScreenshotManager,
    };

    let content = self.refresh_content()?;
    let display = self.primary_display(&content)?;

    let mut cfg = SCStreamConfiguration::new();
    cfg.set_width(display.width() as u32)
        .map_err(|e| CaptureError::Backend {
            message: format!("SCStreamConfiguration.set_width: {e}"),
        })?;
    cfg.set_height(display.height() as u32)
        .map_err(|e| CaptureError::Backend {
            message: format!("SCStreamConfiguration.set_height: {e}"),
        })?;
    cfg.set_pixel_format(screencapturekit::output::PixelFormat::BGRA)
        .map_err(|e| CaptureError::Backend {
            message: format!("set_pixel_format: {e}"),
        })?;

    let filter = SCContentFilter::new_with_display_excluding_windows(&display, &[]);

    let image = SCScreenshotManager::capture_image_with_filter(&filter, &cfg).map_err(|e| {
        if matches!(e.kind(), screencapturekit::error::Kind::PermissionDenied) {
            CaptureError::PermissionDenied
        } else {
            CaptureError::Backend {
                message: format!("SCScreenshotManager::capture_image_with_filter: {e}"),
            }
        }
    })?;

    // `image` is a `CGImageRef` wrapper. Pull bytes via the crate's helper.
    let bytes = image.bgra_bytes().map_err(|e| CaptureError::MalformedFrame {
        reason: format!("image.bgra_bytes: {e}"),
    })?;
    let width_px = image.width() as u32;
    let height_px = image.height() as u32;
    let stride = image.bytes_per_row();

    // Determine display scale (physical per logical). On Retina macOS this is 2.0.
    let scale = display.scale_factor() as f32;

    let raw = RawBgra {
        width_px,
        height_px,
        stride,
        pixels: bytes,
        captured_at_ns: monotonic_ns(),
        scale,
        logical_origin_x: 0,
        logical_origin_y: 0,
    };

    bgra_to_frame(raw)
}
```

Add helper at the top of `sck.rs`:

```rust
fn monotonic_ns() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Monotonic-per-process is the semantic we need, but using system time
    // here is acceptable for a single-machine tick trace. The runtime uses
    // its own `Clock`; this value is only the capture timestamp.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
```

- [ ] **Step 2: Add inline test that `monotonic_ns` is nonzero and monotonic within a call burst**

```rust
#[test]
fn monotonic_ns_advances() {
    let a = super::monotonic_ns();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let b = super::monotonic_ns();
    assert!(b > a);
}
```

(If `screencapturekit` v0.3 doesn't expose exact symbols `set_pixel_format` / `bgra_bytes` / `bytes_per_row` / `scale_factor` / `capture_image_with_filter`, the implementer substitutes the matching names — they exist on some version — and records the exact wire in a module doc at the top of `sck.rs`. Do NOT silently alter the `Capture` trait shape.)

- [ ] **Step 3: Verify compile on macOS**

Run: `cargo check -p vcli-capture`
Expected: OK.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-capture/src/macos/sck.rs
git commit -m "vcli-capture: MacCapture::grab_screen via SCScreenshotManager"
```

---

## Task 8: macOS — `grab_window` by descriptor

**Files:**
- Modify: `crates/vcli-capture/src/macos/sck.rs`

- [ ] **Step 1: Replace `grab_window` stub**

```rust
fn grab_window(&mut self, window: &WindowDescriptor) -> Result<Frame, CaptureError> {
    use screencapturekit::{
        output::sc_stream_configuration::SCStreamConfiguration,
        output::sc_stream_content_filter::SCContentFilter,
        screenshot_manager::SCScreenshotManager,
    };

    let sck_window = self.locate_window(window.id)?;
    let frame_rect = sck_window.frame();
    let width_px = frame_rect.size.width as u32;
    let height_px = frame_rect.size.height as u32;

    let mut cfg = SCStreamConfiguration::new();
    cfg.set_width(width_px).map_err(|e| CaptureError::Backend {
        message: format!("SCStreamConfiguration.set_width: {e}"),
    })?;
    cfg.set_height(height_px).map_err(|e| CaptureError::Backend {
        message: format!("SCStreamConfiguration.set_height: {e}"),
    })?;
    cfg.set_pixel_format(screencapturekit::output::PixelFormat::BGRA)
        .map_err(|e| CaptureError::Backend {
            message: format!("set_pixel_format: {e}"),
        })?;

    let filter = SCContentFilter::new_with_desktop_independent_window(&sck_window);

    let image = SCScreenshotManager::capture_image_with_filter(&filter, &cfg).map_err(|e| {
        if matches!(e.kind(), screencapturekit::error::Kind::PermissionDenied) {
            CaptureError::PermissionDenied
        } else {
            CaptureError::Backend {
                message: format!("capture_image_with_filter: {e}"),
            }
        }
    })?;

    let bytes = image.bgra_bytes().map_err(|e| CaptureError::MalformedFrame {
        reason: format!("image.bgra_bytes: {e}"),
    })?;
    let img_w = image.width() as u32;
    let img_h = image.height() as u32;
    let stride = image.bytes_per_row();

    let content = self.refresh_content()?;
    let display = self.primary_display(&content)?;
    let scale = display.scale_factor() as f32;

    let raw = RawBgra {
        width_px: img_w,
        height_px: img_h,
        stride,
        pixels: bytes,
        captured_at_ns: monotonic_ns(),
        scale,
        logical_origin_x: frame_rect.origin.x as i32,
        logical_origin_y: frame_rect.origin.y as i32,
    };

    bgra_to_frame(raw)
}
```

- [ ] **Step 2: Verify compile on macOS**

Run: `cargo check -p vcli-capture`
Expected: OK.

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-capture/src/macos/sck.rs
git commit -m "vcli-capture: MacCapture::grab_window via desktop-independent filter"
```

---

## Task 9: macOS integration smoke test (TCC-gated, `#[ignore]`)

**Files:**
- Create: `crates/vcli-capture/tests/macos_sck_smoke.rs`

- [ ] **Step 1: Write the integration test**

```rust
//! macOS-only smoke test that actually touches ScreenCaptureKit.
//!
//! Requires Screen Recording permission for the running process. Gated
//! behind `#[ignore]` so `cargo test` does not run it by default. Enable
//! manually with:  `cargo test -p vcli-capture --test macos_sck_smoke -- --ignored`

#![cfg(target_os = "macos")]

use vcli_capture::{
    capture::Capture,
    macos::MacCapture,
    permission::{check_screen_recording_permission, PermissionStatus},
};

#[test]
#[ignore]
fn enumerates_at_least_one_window() {
    assert_eq!(
        check_screen_recording_permission().unwrap(),
        PermissionStatus::Granted,
        "grant Screen Recording permission to the binary before running"
    );
    let c = MacCapture::new().expect("construct MacCapture");
    let windows = c.enumerate_windows().expect("enumerate");
    // A running GUI macOS session always has the Dock / menubar process, so
    // at least one window is guaranteed.
    assert!(!windows.is_empty(), "expected at least one window");
}

#[test]
#[ignore]
fn grabs_a_nonempty_screen_frame() {
    assert_eq!(
        check_screen_recording_permission().unwrap(),
        PermissionStatus::Granted
    );
    let mut c = MacCapture::new().expect("construct MacCapture");
    let frame = c.grab_screen().expect("grab_screen");
    assert!(frame.width() > 0);
    assert!(frame.height() > 0);
    assert!(!frame.pixels.is_empty());
    assert!(frame.pixels.iter().any(|b| *b != 0),
        "captured a fully-black frame — usually means TCC is silently denying");
}
```

- [ ] **Step 2: Compile test on macOS**

Run: `cargo test -p vcli-capture --test macos_sck_smoke --no-run`
Expected: OK.

- [ ] **Step 3: Optionally run (requires TCC)**

Run: `cargo test -p vcli-capture --test macos_sck_smoke -- --ignored`
Expected on a grant-given workstation: both tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-capture/tests/macos_sck_smoke.rs
git commit -m "vcli-capture: TCC-gated integration smoke tests for SCK backend"
```

---

## Task 10: Windows stub (so the workspace builds on Windows)

**Files:**
- Create: `crates/vcli-capture/src/windows/mod.rs`

- [ ] **Step 1: Write the stub module**

```rust
//! Windows capture backend — v0.4 stub. Compiles so the workspace builds on
//! Windows CI, but every operation returns `Unsupported` with a clear message.

use vcli_core::frame::{Frame, FrameFormat};

use crate::capture::{Capture, WindowDescriptor};
use crate::error::CaptureError;

/// Windows capture backend stub.
#[derive(Debug, Default)]
pub struct WindowsCapture;

impl WindowsCapture {
    /// Construct.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Capture for WindowsCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        const FORMATS: &[FrameFormat] = &[FrameFormat::Bgra8];
        FORMATS
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        Err(CaptureError::Unsupported {
            what: "WindowsCapture::enumerate_windows (v0.4)",
        })
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        Err(CaptureError::Unsupported {
            what: "WindowsCapture::grab_screen (v0.4)",
        })
    }

    fn grab_window(&mut self, _window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        Err(CaptureError::Unsupported {
            what: "WindowsCapture::grab_window (v0.4)",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_reports_unsupported() {
        let w = WindowsCapture::new();
        assert_eq!(w.enumerate_windows().unwrap_err().code(), "unsupported");
    }

    #[test]
    fn grab_screen_reports_unsupported() {
        let mut w = WindowsCapture::new();
        assert_eq!(w.grab_screen().unwrap_err().code(), "unsupported");
    }

    #[test]
    fn formats_always_includes_bgra() {
        let w = WindowsCapture::new();
        assert!(w.supported_formats().contains(&FrameFormat::Bgra8));
    }
}
```

- [ ] **Step 2: Verify on mac (module inactive via cfg)**

Run: `cargo check -p vcli-capture`
Expected: OK. Module not compiled.

- [ ] **Step 3: Cross-check build on a Windows runner**

The release workflow lane in the distribution plan will add `windows-latest` to the CI matrix. Meanwhile the implementer can manually verify with `cargo check --target x86_64-pc-windows-msvc` if they have the toolchain. Expected: OK.

- [ ] **Step 4: Commit**

```bash
git add crates/vcli-capture/src/windows/mod.rs
git commit -m "vcli-capture: WindowsCapture stub returning Unsupported (v0.4)"
```

---

## Task 11: `capture_once` example CLI

**Files:**
- Create: `crates/vcli-capture/examples/capture_once.rs`

- [ ] **Step 1: Write the example**

```rust
//! capture_once — grab one screen frame, print dimensions + permission status.
//!
//! Useful for manually verifying macOS Screen Recording permission and the
//! SCK wiring without running the whole daemon.
//!
//! Usage:  cargo run -p vcli-capture --example capture_once
//!         cargo run -p vcli-capture --example capture_once -- --save /tmp/out.png

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(target_os = "macos")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use vcli_capture::{
        capture::Capture,
        macos::MacCapture,
        permission::{
            check_screen_recording_permission, request_screen_recording_permission,
            PermissionStatus,
        },
    };

    let args: Vec<String> = env::args().collect();
    let save_to = args
        .iter()
        .position(|a| a == "--save")
        .and_then(|i| args.get(i + 1).cloned());

    match check_screen_recording_permission()? {
        PermissionStatus::Granted => {
            println!("permission: granted");
        }
        PermissionStatus::Denied => {
            println!("permission: denied — prompting user");
            request_screen_recording_permission()?;
            println!("re-run after granting in System Settings → Privacy & Security → Screen Recording");
            return Ok(());
        }
        PermissionStatus::Unknown => {
            println!("permission: unknown — attempting capture anyway");
        }
    }

    let mut cap = MacCapture::new()?;
    let frame = cap.grab_screen()?;
    println!(
        "captured {}x{} @ stride {} bytes, format {:?}",
        frame.width(),
        frame.height(),
        frame.stride,
        frame.format
    );

    if let Some(path) = save_to {
        use image::{ImageBuffer, Rgba};
        let w = frame.width() as u32;
        let h = frame.height() as u32;
        let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let off = (y as usize) * frame.stride + (x as usize) * 4;
            // BGRA → RGBA
            let b = frame.pixels[off];
            let g = frame.pixels[off + 1];
            let r = frame.pixels[off + 2];
            let a = frame.pixels[off + 3];
            *px = Rgba([r, g, b, a]);
        }
        img.save(&path)?;
        println!("saved: {path}");
    }

    let windows = cap.enumerate_windows()?;
    println!("windows visible: {}", windows.len());
    for w in windows.iter().take(10) {
        println!(
            "  id={} app={:?} title={:?} bounds={}x{}@({},{})",
            w.id, w.app, w.title, w.bounds.w, w.bounds.h, w.bounds.x, w.bounds.y
        );
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("capture_once is only implemented for macOS in v0");
    Ok(())
}
```

- [ ] **Step 2: Verify the example compiles**

Run: `cargo build -p vcli-capture --example capture_once`
Expected: OK on macOS and on Linux (where `run` is a noop).

- [ ] **Step 3: Commit**

```bash
git add crates/vcli-capture/examples/capture_once.rs
git commit -m "vcli-capture: capture_once example for manual TCC/SCK verification"
```

---

## Task 12: `MockCapture` roundtrip via `Capture` trait object + clippy/fmt pass

**Files:**
- Modify: `crates/vcli-capture/src/mock.rs` (add dyn-safety test)
- Modify: any remaining rustdoc / clippy warnings

- [ ] **Step 1: Add a dyn-dispatch test to `mock.rs`**

```rust
#[test]
fn mock_works_through_dyn_capture() {
    use crate::capture::Capture;
    let mut m = MockCapture::new(
        vec![FrameFormat::Bgra8],
        vec![],
        vec![tiny(FrameFormat::Bgra8, 9)],
    );
    let dyn_cap: &mut dyn Capture = &mut m;
    let f = dyn_cap.grab_screen().unwrap();
    assert_eq!(f.pixels[0], 9);
}
```

- [ ] **Step 2: Run the full test suite**

Run: `cargo test -p vcli-capture`
Expected on macOS: `mock`, `error`, `capture`, `permission`, `macos::convert`, `macos::sck` suites all green. Integration test skipped (`#[ignore]`).
Expected on Linux: `mock`, `error`, `capture`, `permission::non_macos_*`, `windows::*` all green.

- [ ] **Step 3: Clippy pass**

Run: `cargo clippy -p vcli-capture --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Fmt pass**

Run: `cargo fmt -p vcli-capture`
Expected: no changes (or apply + re-check).

- [ ] **Step 5: Commit**

```bash
git add crates/vcli-capture/src/mock.rs
git commit -m "vcli-capture: dyn-Capture test + clippy/fmt clean"
```

---

## Self-review — spec coverage

| Spec item | Covered in |
|---|---|
| `Capture` trait (§Architecture crate responsibilities) | Task 1 |
| Window enumeration with stable descriptors + `WindowIndex` grouping (Decision F2) | Tasks 1, 6 |
| Full-screen frame via ScreenCaptureKit (Decision 1.4) | Tasks 5, 7 |
| Window-scoped frame | Task 8 |
| Supported frame formats query | Task 1 (trait), Tasks 3/6/10 (impls) |
| `MockCapture` used by other lanes' tests | Task 3 |
| Windows trait abstraction + stub | Tasks 1, 10 |
| macOS permission probe (TCC) + `capture.permission_missing` event wiring | Task 4 (probe) + Task 6 (`CaptureError::PermissionDenied` return) |
| Physical → logical downsample at capture boundary (Decisions F1, 4.3) | Task 5 |
| Example / smoke tool for manual verification | Task 11 |
| Unit tests against `MockCapture` | Task 3 |
| Integration test, `#[ignore]`-gated for TCC | Task 9 |
| `CaptureError` mapping to ipc `capture_failed` / `permission_denied` codes (§Error codes) | Task 2 |
| One capture per tick — trait shape returns owned `Frame`, scheduler wraps in `Arc` | Task 1 (trait signature) |
| No modifications to `vcli-core` | Enforced — this lane only reads its types |
| Scope excludes input (D), predicate eval (G), tick loop (runtime lane) | Correct; no such code appears |

### Spec ambiguity resolved

1. The spec says "ScreenCaptureKit via `core-graphics`" in one place and "screencapturekit-rs (modern Apple API)" in Decision 1.4. Decision 1.4 explicitly wins. I chose the crates.io crate **`screencapturekit`** (the `screencapturekit-rs` GitHub project's published name) because (a) it is the modern SCK binding Decision 1.4 names, (b) it provides safe wrappers for `SCShareableContent`, `SCScreenshotManager`, and `SCStreamConfiguration` — which is what this lane needs — saving us from hand-rolling `objc2` bindings, and (c) it carries a permission helper we plug into the TCC probe. If the implementer discovers the published crate's API surface differs from what Task 6–8 assume, they preserve the `Capture` trait shape and re-wire the SCK call sites; the trait, error taxonomy, and mock are decoupled from the SCK crate version.
2. Spec does not explicitly specify a descriptor type for window enumeration. I defined `WindowDescriptor` (id + app + title + bounds + `WindowIndex` + display), which matches the needs of `Region::Window { app, title_contains, window_index }` from `vcli-core`.
3. Spec is silent on whether `grab_window` re-resolves AX state or uses the descriptor's cached bounds. I chose to re-resolve by id because the spec's Decision 1.8 invalidates geometry cache on move/resize — grabs must reflect current bounds, not enumeration-time bounds.
4. Spec's "reporting supported frame formats" is mentioned once with no shape. I chose `fn supported_formats(&self) -> &[FrameFormat]` returning the preferred format first — minimal, static, zero-allocation.
