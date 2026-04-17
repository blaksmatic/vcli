//! Low-level accessors that bridge `vcli-core::Frame` (BGRA8 / RGBA8,
//! possibly with stride padding) to `image::RgbImage` slices used by
//! `imageproc` and the `pixel_diff` dHash.
//!
//! All evaluators go through this module so the BGRA↔RGB swizzle and the
//! Frame-bounds-vs-region clipping logic lives in exactly one place.

use image::{ImageBuffer, Rgb, RgbImage};

use vcli_core::geom::Rect;
use vcli_core::{Frame, FrameFormat};

use crate::error::{PerceptionError, Result};

/// Read the RGB value at `(x, y)` in frame-local (not screen) coords.
///
/// # Errors
///
/// Returns `RegionOutOfBounds` if the coordinate is outside the frame.
pub fn pixel_rgb(frame: &Frame, x: i32, y: i32) -> Result<[u8; 3]> {
    if x < 0 || y < 0 || x >= frame.width() || y >= frame.height() {
        return Err(PerceptionError::RegionOutOfBounds);
    }
    // Safety: x and y are both non-negative (checked above).
    #[allow(clippy::cast_sign_loss)]
    let ux = x as usize;
    #[allow(clippy::cast_sign_loss)]
    let uy = y as usize;
    let bpp = frame.format.bytes_per_pixel();
    let offset = uy.saturating_mul(frame.stride) + ux.saturating_mul(bpp);
    let bytes = &frame.pixels[offset..offset + bpp];
    Ok(match frame.format {
        // BGRA8: B, G, R, A
        FrameFormat::Bgra8 => [bytes[2], bytes[1], bytes[0]],
        // RGBA8: R, G, B, A
        FrameFormat::Rgba8 => [bytes[0], bytes[1], bytes[2]],
    })
}

/// Crop a rectangle out of the frame and return it as an `RgbImage`.
/// `region_abs` is in absolute screen coords; this function translates
/// them into frame-local coords using `frame.bounds.top_left()`.
///
/// If the region partially overlaps, returns the overlap. If there is
/// zero overlap, returns `RegionOutOfBounds`.
///
/// # Errors
///
/// `RegionOutOfBounds` if the region does not intersect the frame.
pub fn crop_rgb(frame: &Frame, region_abs: Rect) -> Result<RgbImage> {
    let fb = frame.bounds;
    let x0 = region_abs.x.max(fb.x);
    let y0 = region_abs.y.max(fb.y);
    let x1 = (region_abs.x + region_abs.w).min(fb.x + fb.w);
    let y1 = (region_abs.y + region_abs.h).min(fb.y + fb.h);
    if x1 <= x0 || y1 <= y0 {
        return Err(PerceptionError::RegionOutOfBounds);
    }
    // w and h are differences of clamped i32 coords, both non-negative.
    #[allow(clippy::cast_sign_loss)]
    let w = (x1 - x0) as u32;
    #[allow(clippy::cast_sign_loss)]
    let h = (y1 - y0) as u32;
    let mut out = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(w, h);
    for row in 0..h {
        for col in 0..w {
            // col and row are u32 from 0..w/h; guaranteed to fit in i32 for
            // practical frame dimensions (< 2^31 pixels).
            #[allow(clippy::cast_possible_wrap)]
            let fx = (x0 - fb.x) + col as i32;
            #[allow(clippy::cast_possible_wrap)]
            let fy = (y0 - fb.y) + row as i32;
            let rgb = pixel_rgb(frame, fx, fy)?;
            out.put_pixel(col, row, Rgb(rgb));
        }
    }
    Ok(out)
}

/// Convert the full frame into an owned `RgbImage`. Used by
/// `TemplateEvaluator` when the region covers the whole frame.
///
/// # Errors
///
/// Propagates `pixel_rgb` errors (unreachable for in-bounds iteration).
pub fn frame_to_rgb(frame: &Frame) -> Result<RgbImage> {
    // width() and height() return i32 which are non-negative for valid frames.
    #[allow(clippy::cast_sign_loss)]
    let w = frame.width() as u32;
    #[allow(clippy::cast_sign_loss)]
    let h = frame.height() as u32;
    let mut out = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(w, h);
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            let rgb = pixel_rgb(frame, x, y)?;
            // x and y are non-negative (0..positive_i32).
            #[allow(clippy::cast_sign_loss)]
            let px = x as u32;
            #[allow(clippy::cast_sign_loss)]
            let py = y as u32;
            out.put_pixel(px, py, Rgb(rgb));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Build a 4×2 BGRA8 frame where pixel (x, y) = RGB (10·x, 20·y, x + y).
    fn bgra_test_frame() -> Frame {
        let w = 4usize;
        let h = 2usize;
        let stride = w * 4;
        let mut pixels = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let off = y * stride + x * 4;
                // Test values are small (x<4, y<2) so casts are safe.
                #[allow(clippy::cast_possible_truncation)]
                {
                    pixels[off] = (x + y) as u8; // B
                    pixels[off + 1] = (20 * y) as u8; // G
                    pixels[off + 2] = (10 * x) as u8; // R
                }
                pixels[off + 3] = 255;
            }
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        Frame::new(
            FrameFormat::Bgra8,
            Rect {
                x: 0,
                y: 0,
                w: w as i32,
                h: h as i32,
            },
            stride,
            Arc::from(pixels),
            0,
        )
    }

    #[test]
    fn pixel_rgb_bgra_swizzles_to_rgb() {
        let f = bgra_test_frame();
        assert_eq!(pixel_rgb(&f, 0, 0).unwrap(), [0, 0, 0]);
        assert_eq!(pixel_rgb(&f, 3, 1).unwrap(), [30, 20, 4]);
    }

    #[test]
    fn pixel_rgb_out_of_bounds_errors() {
        let f = bgra_test_frame();
        assert!(matches!(
            pixel_rgb(&f, -1, 0),
            Err(PerceptionError::RegionOutOfBounds)
        ));
        assert!(matches!(
            pixel_rgb(&f, 0, 10),
            Err(PerceptionError::RegionOutOfBounds)
        ));
    }

    #[test]
    fn crop_rgb_returns_correct_dimensions() {
        let f = bgra_test_frame();
        let crop = crop_rgb(
            &f,
            Rect {
                x: 1,
                y: 0,
                w: 2,
                h: 2,
            },
        )
        .unwrap();
        assert_eq!(crop.width(), 2);
        assert_eq!(crop.height(), 2);
        // (1, 0) = R 10, G 0, B 1
        assert_eq!(crop.get_pixel(0, 0).0, [10, 0, 1]);
    }

    #[test]
    fn crop_rgb_clips_to_frame_bounds() {
        let f = bgra_test_frame();
        let crop = crop_rgb(
            &f,
            Rect {
                x: 3,
                y: 1,
                w: 10,
                h: 10,
            },
        )
        .unwrap();
        assert_eq!(crop.width(), 1);
        assert_eq!(crop.height(), 1);
    }

    #[test]
    fn crop_rgb_zero_overlap_errors() {
        let f = bgra_test_frame();
        let crop = crop_rgb(
            &f,
            Rect {
                x: 100,
                y: 100,
                w: 10,
                h: 10,
            },
        );
        assert!(matches!(crop, Err(PerceptionError::RegionOutOfBounds)));
    }

    #[test]
    fn frame_to_rgb_has_correct_dimensions() {
        let f = bgra_test_frame();
        let img = frame_to_rgb(&f).unwrap();
        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 2);
    }
}
