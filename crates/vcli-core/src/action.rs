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
