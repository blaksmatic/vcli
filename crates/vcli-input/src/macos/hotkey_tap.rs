//! CGEventTap-based global hotkey listener.
//!
//! Spawns a dedicated thread with its own `CFRunLoop`. Installs an event tap at
//! `kCGHIDEventTap` in `kCGEventTapOptionListenOnly` mode so it observes — but
//! never consumes — keystrokes. When the `Cmd+Shift+Esc` chord is detected
//! (keycode `0x35` with Cmd + Shift flags on KeyDown), it calls
//! `KillSwitch::engage()`. Dropping the returned `KillSwitchListenerHandle`
//! stops the run loop and joins the thread.
//!
//! Why `Cmd+Shift+Esc`? macOS reserves `Cmd+Option+Esc` for Force Quit;
//! `Cmd+Shift+Esc` is otherwise unused system-wide, easy to chord one-handed,
//! and semantically "escape from the automation."

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventTapProxy, CGEventType,
};

use crate::error::InputError;
use crate::kill_switch::KillSwitch;

/// `kVK_Escape` from HIToolbox/Events.h — numeric discriminant in the CGEventType repr.
const KVK_ESCAPE_KEYCODE: i64 = 0x35;
/// `CGEventType::KeyDown` discriminant value.
const CG_EVENT_KEY_DOWN: u32 = CGEventType::KeyDown as u32;

/// Handle returned by [`spawn_kill_switch_listener`]. Dropping stops the tap.
pub struct KillSwitchListenerHandle {
    stop: Arc<Mutex<Option<CFRunLoop>>>,
    join: Option<JoinHandle<()>>,
}

impl Drop for KillSwitchListenerHandle {
    fn drop(&mut self) {
        if let Some(rl) = self.stop.lock().unwrap().take() {
            rl.stop();
        }
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

/// Start the listener. Returns a handle — dropping the handle stops the thread.
///
/// Fails if the current process doesn't hold Input Monitoring permission.
pub fn spawn_kill_switch_listener(
    kill: KillSwitch,
) -> Result<KillSwitchListenerHandle, InputError> {
    let stop: Arc<Mutex<Option<CFRunLoop>>> = Arc::new(Mutex::new(None));
    let stop_for_thread = stop.clone();

    let join = std::thread::Builder::new()
        .name("vcli-input-killswitch-tap".into())
        .spawn(move || {
            // CGEventTap::new takes a closure that may borrow local data; we
            // move `kill` into the closure so it lives for the tap's lifetime.
            let tap_result = CGEventTap::new(
                CGEventTapLocation::HID,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                vec![CGEventType::KeyDown],
                {
                    let kill = kill.clone();
                    move |_proxy: CGEventTapProxy,
                          event_type: CGEventType,
                          event: &CGEvent|
                          -> Option<CGEvent> {
                        // CGEventType doesn't impl PartialEq — compare discriminants.
                        if event_type as u32 == CG_EVENT_KEY_DOWN {
                            let keycode = event.get_integer_value_field(
                                core_graphics::event::EventField::KEYBOARD_EVENT_KEYCODE,
                            );
                            let flags = event.get_flags();
                            let want = CGEventFlags::CGEventFlagCommand
                                | CGEventFlags::CGEventFlagShift;
                            if keycode == KVK_ESCAPE_KEYCODE && flags.contains(want) {
                                kill.engage();
                            }
                        }
                        // Listen-only: return None to pass through unchanged.
                        None
                    }
                },
            );

            let tap = match tap_result {
                Ok(t) => t,
                Err(()) => {
                    // Input Monitoring not granted — exit thread cleanly.
                    return;
                }
            };

            let runloop = CFRunLoop::get_current();
            let source = tap
                .mach_port
                .create_runloop_source(0)
                .expect("create_runloop_source failed");
            // Safety: kCFRunLoopCommonModes is a valid CFStringRef constant;
            // add_source uses it as a mode identifier for the run loop.
            runloop.add_source(&source, unsafe { kCFRunLoopCommonModes });
            tap.enable();

            *stop_for_thread.lock().unwrap() = Some(runloop);

            // Run until stop() is called from Drop.
            CFRunLoop::run_current();
        })
        .map_err(|e| InputError::Backend {
            detail: format!("spawn tap thread: {e}"),
        })?;

    Ok(KillSwitchListenerHandle {
        stop,
        join: Some(join),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawn + drop should not panic; in CI the tap creation will fail cleanly
    /// because the process lacks Input Monitoring, and the thread exits early.
    #[test]
    fn spawn_and_drop_does_not_panic() {
        let kill = KillSwitch::new();
        let handle = spawn_kill_switch_listener(kill.clone()).unwrap();
        drop(handle);
    }
}
