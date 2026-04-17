//! Mapping from the vcli canonical key-name vocabulary to platform virtual
//! keycodes. Kept cross-platform so the Windows backend has the same parsing.
//!
//! Canonical names are lowercase ASCII, e.g. `"a"`, `"return"`, `"space"`,
//! `"tab"`, `"escape"`, `"left"`, `"right"`, `"up"`, `"down"`, `"f1"`..`"f12"`,
//! `"backspace"`, `"delete"`, `"home"`, `"end"`, `"page_up"`, `"page_down"`.

use crate::error::InputError;

/// Parsed canonical key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalKey {
    /// Single printable ASCII character (after normalization).
    Char(char),
    /// Named special key.
    Named(NamedKey),
}

/// Named special keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NamedKey {
    /// Return / Enter.
    Return,
    /// Tab.
    Tab,
    /// Spacebar.
    Space,
    /// Escape.
    Escape,
    /// Backspace.
    Backspace,
    /// Forward delete.
    Delete,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Function keys 1..12.
    Function(u8),
}

/// Parse a canonical name. Returns `UnknownKey` on anything we don't recognize.
pub fn parse(name: &str) -> Result<CanonicalKey, InputError> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty() {
        return Err(InputError::UnknownKey(name.to_owned()));
    }

    if let Some(rest) = n.strip_prefix('f') {
        if let Ok(num) = rest.parse::<u8>() {
            if (1..=12).contains(&num) {
                return Ok(CanonicalKey::Named(NamedKey::Function(num)));
            }
        }
    }

    let named = match n.as_str() {
        "return" | "enter" => NamedKey::Return,
        "tab" => NamedKey::Tab,
        "space" => NamedKey::Space,
        "escape" | "esc" => NamedKey::Escape,
        "backspace" => NamedKey::Backspace,
        "delete" | "forward_delete" => NamedKey::Delete,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "page_up" | "pageup" => NamedKey::PageUp,
        "page_down" | "pagedown" => NamedKey::PageDown,
        _ => {
            if n.chars().count() == 1 {
                return Ok(CanonicalKey::Char(n.chars().next().unwrap()));
            }
            return Err(InputError::UnknownKey(name.to_owned()));
        }
    };
    Ok(CanonicalKey::Named(named))
}

/// Translate a parsed key into a macOS virtual keycode (from
/// `HIToolbox/Events.h` / `kVK_*`). Returns `None` for code-points that can't
/// be typed via a virtual key (fallback path: type_text via Unicode events).
#[cfg(target_os = "macos")]
#[must_use]
pub fn macos_keycode(key: CanonicalKey) -> Option<u16> {
    use CanonicalKey::{Char, Named};
    use NamedKey::{
        Backspace, Delete, Down, End, Escape, Function, Home, Left, PageDown, PageUp, Return,
        Right, Space, Tab, Up,
    };
    Some(match key {
        Named(Return) => 0x24,
        Named(Tab) => 0x30,
        Named(Space) => 0x31,
        Named(Backspace) => 0x33,
        Named(Escape) => 0x35,
        Named(Delete) => 0x75,
        Named(Home) => 0x73,
        Named(End) => 0x77,
        Named(PageUp) => 0x74,
        Named(PageDown) => 0x79,
        Named(Left) => 0x7B,
        Named(Right) => 0x7C,
        Named(Down) => 0x7D,
        Named(Up) => 0x7E,
        Named(Function(1)) => 0x7A,
        Named(Function(2)) => 0x78,
        Named(Function(3)) => 0x63,
        Named(Function(4)) => 0x76,
        Named(Function(5)) => 0x60,
        Named(Function(6)) => 0x61,
        Named(Function(7)) => 0x62,
        Named(Function(8)) => 0x64,
        Named(Function(9)) => 0x65,
        Named(Function(10)) => 0x6D,
        Named(Function(11)) => 0x67,
        Named(Function(12)) => 0x6F,
        Named(Function(_)) => return None,
        Char(c) => return ascii_to_macos_keycode(c),
    })
}

#[cfg(target_os = "macos")]
fn ascii_to_macos_keycode(c: char) -> Option<u16> {
    // kVK_ANSI_* keycodes for US layout (HIToolbox/Events.h).
    Some(match c {
        'a' => 0x00,
        's' => 0x01,
        'd' => 0x02,
        'f' => 0x03,
        'h' => 0x04,
        'g' => 0x05,
        'z' => 0x06,
        'x' => 0x07,
        'c' => 0x08,
        'v' => 0x09,
        'b' => 0x0B,
        'q' => 0x0C,
        'w' => 0x0D,
        'e' => 0x0E,
        'r' => 0x0F,
        'y' => 0x10,
        't' => 0x11,
        '1' => 0x12,
        '2' => 0x13,
        '3' => 0x14,
        '4' => 0x15,
        '6' => 0x16,
        '5' => 0x17,
        '=' => 0x18,
        '9' => 0x19,
        '7' => 0x1A,
        '-' => 0x1B,
        '8' => 0x1C,
        '0' => 0x1D,
        ']' => 0x1E,
        'o' => 0x1F,
        'u' => 0x20,
        '[' => 0x21,
        'i' => 0x22,
        'p' => 0x23,
        'l' => 0x25,
        'j' => 0x26,
        '\'' => 0x27,
        'k' => 0x28,
        ';' => 0x29,
        '\\' => 0x2A,
        ',' => 0x2B,
        '/' => 0x2C,
        'n' => 0x2D,
        'm' => 0x2E,
        '.' => 0x2F,
        '`' => 0x32,
        _ => return None,
    })
}

/// Non-macOS stub so the symbol exists on every platform but always returns None.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn macos_keycode(_key: CanonicalKey) -> Option<u16> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_parse() {
        assert_eq!(parse("return").unwrap(), CanonicalKey::Named(NamedKey::Return));
        assert_eq!(parse("Enter").unwrap(), CanonicalKey::Named(NamedKey::Return));
        assert_eq!(parse("esc").unwrap(), CanonicalKey::Named(NamedKey::Escape));
        assert_eq!(parse("page_up").unwrap(), CanonicalKey::Named(NamedKey::PageUp));
    }

    #[test]
    fn function_keys_parse_in_range() {
        assert_eq!(
            parse("f1").unwrap(),
            CanonicalKey::Named(NamedKey::Function(1))
        );
        assert_eq!(
            parse("F12").unwrap(),
            CanonicalKey::Named(NamedKey::Function(12))
        );
        assert!(matches!(parse("f0"), Err(InputError::UnknownKey(_))));
        assert!(matches!(parse("f13"), Err(InputError::UnknownKey(_))));
    }

    #[test]
    fn single_chars_parse() {
        assert_eq!(parse("s").unwrap(), CanonicalKey::Char('s'));
        assert_eq!(parse("1").unwrap(), CanonicalKey::Char('1'));
    }

    #[test]
    fn empty_and_garbage_reject() {
        assert!(matches!(parse(""), Err(InputError::UnknownKey(_))));
        assert!(matches!(parse("blargh"), Err(InputError::UnknownKey(_))));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_keycode_known_keys() {
        assert_eq!(macos_keycode(parse("return").unwrap()), Some(0x24));
        assert_eq!(macos_keycode(parse("a").unwrap()), Some(0x00));
        assert_eq!(macos_keycode(parse("f1").unwrap()), Some(0x7A));
    }
}
