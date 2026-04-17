//! macOS-only backend modules. Gated on `cfg(target_os = "macos")` at the
//! crate root.

pub mod cg_events;
pub mod cg_sink;
pub mod cg_typing;
pub mod hotkey_tap;
pub mod tcc;

pub use cg_sink::CGEventInputSink;
pub use hotkey_tap::{spawn_kill_switch_listener, KillSwitchListenerHandle};
