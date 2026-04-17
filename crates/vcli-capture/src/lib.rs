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
