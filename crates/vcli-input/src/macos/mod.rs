//! macOS-only backend modules. Gated on `cfg(target_os = "macos")` at the
//! crate root.

pub mod tcc;
pub mod cg_events;
pub mod cg_typing;
pub mod cg_sink;
pub mod hotkey_tap;

pub use cg_sink::CGEventInputSink;
pub use hotkey_tap::{spawn_kill_switch_listener, KillSwitchListenerHandle};
