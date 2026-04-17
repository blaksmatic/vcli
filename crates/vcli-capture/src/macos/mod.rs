//! macOS capture backend. Enabled on `target_os = "macos"` only.

pub mod convert;
pub mod sck;

pub use sck::MacCapture;
