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
            rect: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 50,
            },
        };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(
            j,
            r#"{"kind":"absolute","box":{"x":0,"y":0,"w":100,"h":50}}"#
        );
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
            Region::RelativeTo {
                predicate,
                anchor,
                offset,
                size,
            } => {
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
