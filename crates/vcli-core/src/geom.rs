//! Geometric primitives: integer Point and Rect.
//!
//! Coordinates are in logical (1x) pixels. Capture converts physical→logical
//! at the capture boundary per Decision F1/4.3; everything above `vcli-capture`
//! operates in logical space.

use serde::{Deserialize, Serialize};

/// A point in logical (1x) pixels. Top-left origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate (pixels, left→right).
    pub x: i32,
    /// Vertical coordinate (pixels, top→bottom).
    pub y: i32,
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge (pixels).
    pub x: i32,
    /// Top edge (pixels).
    pub y: i32,
    /// Width (pixels, non-negative in valid rects).
    pub w: i32,
    /// Height (pixels, non-negative in valid rects).
    pub h: i32,
}

impl Rect {
    /// Center point of the rectangle (integer-rounded toward zero).
    #[must_use]
    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2,
            y: self.y + self.h / 2,
        }
    }

    /// Top-left corner.
    #[must_use]
    pub fn top_left(&self) -> Point {
        Point { x: self.x, y: self.y }
    }

    /// Whether this rect contains the given point (inclusive of top/left, exclusive of bottom/right).
    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_center_is_correct_for_even_dims() {
        let r = Rect { x: 0, y: 0, w: 100, h: 40 };
        assert_eq!(r.center(), Point { x: 50, y: 20 });
    }

    #[test]
    fn rect_center_rounds_toward_zero_for_odd_dims() {
        let r = Rect { x: 10, y: 20, w: 5, h: 7 };
        assert_eq!(r.center(), Point { x: 12, y: 23 });
    }

    #[test]
    fn rect_top_left_reports_origin() {
        let r = Rect { x: 3, y: 4, w: 10, h: 10 };
        assert_eq!(r.top_left(), Point { x: 3, y: 4 });
    }

    #[test]
    fn rect_contains_is_inclusive_top_left_exclusive_bottom_right() {
        let r = Rect { x: 0, y: 0, w: 10, h: 10 };
        assert!(r.contains(Point { x: 0, y: 0 }));
        assert!(r.contains(Point { x: 9, y: 9 }));
        assert!(!r.contains(Point { x: 10, y: 5 }));
        assert!(!r.contains(Point { x: 5, y: 10 }));
    }

    #[test]
    fn point_serde_roundtrip() {
        let p = Point { x: -5, y: 7 };
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(j, r#"{"x":-5,"y":7}"#);
        let back: Point = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn rect_serde_roundtrip() {
        let r = Rect { x: 1, y: 2, w: 3, h: 4 };
        let j = serde_json::to_string(&r).unwrap();
        let back: Rect = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }
}
