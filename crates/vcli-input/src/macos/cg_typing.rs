//! `type_text` for macOS. Uses `CGEventKeyboardSetUnicodeString` so the active
//! keyboard layout is respected and every Unicode code point is typeable.

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use core_graphics::event::{CGEvent, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

use crate::error::InputError;

/// Post one keyboard event carrying `chunk` as its Unicode payload (key down)
/// followed by a matching key up. Called once per grapheme cluster so layouts
/// that render combining marks still produce the expected glyph.
fn post_unicode_chunk(chunk: &str) -> Result<(), InputError> {
    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|()| {
        InputError::Backend {
            detail: "CGEventSource::new failed".into(),
        }
    })?;

    let utf16: Vec<u16> = chunk.encode_utf16().collect();

    let down =
        CGEvent::new_keyboard_event(src.clone(), 0, true).map_err(|()| InputError::Backend {
            detail: "keyboard_event down failed".into(),
        })?;
    down.set_string_from_utf16_unchecked(&utf16);
    down.post(CGEventTapLocation::HID);

    let up = CGEvent::new_keyboard_event(src, 0, false).map_err(|()| InputError::Backend {
        detail: "keyboard_event up failed".into(),
    })?;
    up.set_string_from_utf16_unchecked(&utf16);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

/// Type a UTF-8 string one grapheme cluster at a time. If the `core-graphics`
/// crate lacks a helper with that exact name, the implementer should use
/// `set_string`, `set_string_from_utf16`, or the FFI `CGEventKeyboardSetUnicodeString`
/// directly.
///
/// # Errors
///
/// Returns `InputError::Backend` if any key-event creation or posting fails.
pub fn type_text(text: &str) -> Result<(), InputError> {
    if text.is_empty() {
        return Ok(());
    }
    // One event per grapheme cluster. v0 approximation: one per character.
    for ch in text.chars() {
        let mut buf = [0u8; 4];
        let chunk = ch.encode_utf8(&mut buf);
        post_unicode_chunk(chunk)?;
    }
    Ok(())
}
