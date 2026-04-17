//! `Frame` — one capture result passed to perception. Always in logical-pixel
//! resolution (Decision F1 / 4.3). Not serialized (frames are never persisted).

use std::sync::Arc;

use crate::geom::Rect;

/// Pixel format of a frame buffer. v0 emits BGRA8 from macOS `ScreenCaptureKit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    /// 4 bytes per pixel, order B, G, R, A. Stride may include row padding.
    Bgra8,
    /// 4 bytes per pixel, order R, G, B, A.
    Rgba8,
}

impl FrameFormat {
    /// Bytes per pixel for this format.
    #[must_use]
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgra8 | Self::Rgba8 => 4,
        }
    }
}

/// A captured screen frame. Shared via `Arc<Frame>` across the tick's perception work.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Pixel format.
    pub format: FrameFormat,
    /// Bounds of the frame in logical pixels. `bounds.top_left()` is where
    /// this buffer originates on the logical desktop.
    pub bounds: Rect,
    /// Row stride in bytes. Usually `bounds.w * bytes_per_pixel`, but some
    /// backends add padding.
    pub stride: usize,
    /// Raw pixel bytes. Length ≥ `stride * bounds.h`.
    pub pixels: Arc<[u8]>,
    /// Monotonic timestamp in nanoseconds since an unspecified epoch.
    pub captured_at_ns: u64,
}

impl Frame {
    /// Convenience constructor.
    ///
    /// # Panics
    ///
    /// Panics if `pixels.len() < stride * bounds.h`.
    #[must_use]
    pub fn new(
        format: FrameFormat,
        bounds: Rect,
        stride: usize,
        pixels: Arc<[u8]>,
        captured_at_ns: u64,
    ) -> Self {
        let h = usize::try_from(bounds.h).unwrap_or(0);
        let needed = stride.saturating_mul(h);
        assert!(
            pixels.len() >= needed,
            "frame buffer too small: have {}, need {needed}",
            pixels.len()
        );
        Self { format, bounds, stride, pixels, captured_at_ns }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> i32 {
        self.bounds.w
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> i32 {
        self.bounds.h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frame {
        Frame::new(
            FrameFormat::Bgra8,
            Rect { x: 0, y: 0, w: 4, h: 2 },
            4 * 4,
            vec![0u8; 4 * 4 * 2].into(),
            123,
        )
    }

    #[test]
    fn bytes_per_pixel_matches_format() {
        assert_eq!(FrameFormat::Bgra8.bytes_per_pixel(), 4);
        assert_eq!(FrameFormat::Rgba8.bytes_per_pixel(), 4);
    }

    #[test]
    fn new_stores_inputs_verbatim() {
        let f = sample();
        assert_eq!(f.width(), 4);
        assert_eq!(f.height(), 2);
        assert_eq!(f.stride, 16);
        assert_eq!(f.format, FrameFormat::Bgra8);
        assert_eq!(f.captured_at_ns, 123);
    }

    #[test]
    #[should_panic(expected = "frame buffer too small")]
    fn new_panics_on_too_small_buffer() {
        let _ = Frame::new(
            FrameFormat::Bgra8,
            Rect { x: 0, y: 0, w: 100, h: 100 },
            400,
            vec![0u8; 10].into(),
            0,
        );
    }
}
