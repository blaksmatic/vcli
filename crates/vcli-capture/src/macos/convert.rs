//! Pure BGRA / physical → logical converters. No FFI. Unit-tested with
//! synthetic buffers so platform-free CI exercises the pixel math.

use std::sync::Arc;

use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;

use crate::error::CaptureError;

/// Parameters for a raw BGRA8 buffer emitted by a backend.
#[derive(Debug, Clone)]
pub struct RawBgra {
    /// Physical-pixel width.
    pub width_px: u32,
    /// Physical-pixel height.
    pub height_px: u32,
    /// Row stride in bytes. May exceed `width_px * 4` for alignment padding.
    pub stride: usize,
    /// Physical pixel bytes. Length must be ≥ `stride * height_px`.
    pub pixels: Vec<u8>,
    /// Monotonic ns timestamp.
    pub captured_at_ns: u64,
    /// Factor to downsample to logical 1x. 2.0 for Retina, 1.0 for non-HiDPI.
    pub scale: f32,
    /// Logical origin (top-left of the captured region in logical desktop coords).
    pub logical_origin_x: i32,
    /// Logical origin y.
    pub logical_origin_y: i32,
}

/// Convert a raw BGRA physical-pixel buffer into a logical-resolution `Frame`.
///
/// For `scale == 1.0` the bytes are wrapped with no copying aside from the
/// `Arc`. For `scale == 2.0` (Retina) we perform a simple 2:1 box-filter
/// downsample so templates authored at logical resolution match.
///
/// # Errors
///
/// Returns [`CaptureError::MalformedFrame`] if:
/// - `pixels.len() < stride * height_px`
/// - `stride < width_px * 4`
/// - `scale` not in {1.0, 2.0} (v0 supported scales; fractional DPI deferred)
pub fn bgra_to_frame(raw: RawBgra) -> Result<Frame, CaptureError> {
    let need = raw
        .stride
        .checked_mul(raw.height_px as usize)
        .ok_or_else(|| CaptureError::MalformedFrame {
            reason: "stride * height overflow".into(),
        })?;
    if raw.pixels.len() < need {
        return Err(CaptureError::MalformedFrame {
            reason: format!(
                "buffer len {} < stride*height {}",
                raw.pixels.len(),
                need
            ),
        });
    }
    if raw.stride < (raw.width_px as usize) * 4 {
        return Err(CaptureError::MalformedFrame {
            reason: format!(
                "stride {} less than width*4 {}",
                raw.stride,
                raw.width_px * 4
            ),
        });
    }

    let scale = raw.scale;
    let (out_w, out_h, out_stride, out_pixels) = if (scale - 1.0_f32).abs() < f32::EPSILON {
        (
            raw.width_px as i32,
            raw.height_px as i32,
            raw.stride,
            raw.pixels,
        )
    } else if (scale - 2.0_f32).abs() < f32::EPSILON {
        downsample_2x(&raw)
    } else {
        return Err(CaptureError::MalformedFrame {
            reason: format!("unsupported scale {scale}"),
        });
    };

    let bounds = Rect {
        x: raw.logical_origin_x,
        y: raw.logical_origin_y,
        w: out_w,
        h: out_h,
    };

    let bytes: Arc<[u8]> = out_pixels.into();
    Ok(Frame::new(
        FrameFormat::Bgra8,
        bounds,
        out_stride,
        bytes,
        raw.captured_at_ns,
    ))
}

/// 2:1 box-filter downsample of a BGRA buffer. Returns
/// `(out_w, out_h, out_stride, out_pixels)`. Output stride = out_w * 4.
fn downsample_2x(raw: &RawBgra) -> (i32, i32, usize, Vec<u8>) {
    let out_w = (raw.width_px / 2) as usize;
    let out_h = (raw.height_px / 2) as usize;
    let out_stride = out_w * 4;
    let mut out = vec![0u8; out_stride * out_h];
    for y in 0..out_h {
        let src_y0 = y * 2;
        let src_y1 = src_y0 + 1;
        let row0 = &raw.pixels
            [src_y0 * raw.stride..src_y0 * raw.stride + raw.width_px as usize * 4];
        let row1 = &raw.pixels
            [src_y1 * raw.stride..src_y1 * raw.stride + raw.width_px as usize * 4];
        for x in 0..out_w {
            let sx = x * 2 * 4;
            let idx = y * out_stride + x * 4;
            // Box average 4 source pixels per channel.
            for c in 0..4 {
                let sum = u32::from(row0[sx + c])
                    + u32::from(row0[sx + 4 + c])
                    + u32::from(row1[sx + c])
                    + u32::from(row1[sx + 4 + c]);
                #[allow(clippy::cast_possible_truncation)]
                {
                    out[idx + c] = (sum / 4) as u8;
                }
            }
        }
    }
    (out_w as i32, out_h as i32, out_stride, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(width: u32, height: u32, stride: usize, fill: u8, scale: f32) -> RawBgra {
        RawBgra {
            width_px: width,
            height_px: height,
            stride,
            pixels: vec![fill; stride * height as usize],
            captured_at_ns: 1,
            scale,
            logical_origin_x: 0,
            logical_origin_y: 0,
        }
    }

    #[test]
    fn scale_1_passes_through_dims() {
        let r = raw(8, 4, 32, 0xAB, 1.0);
        let f = bgra_to_frame(r).unwrap();
        assert_eq!(f.width(), 8);
        assert_eq!(f.height(), 4);
        assert_eq!(f.stride, 32);
        assert_eq!(f.format, FrameFormat::Bgra8);
        assert_eq!(f.pixels[0], 0xAB);
    }

    #[test]
    fn scale_2_halves_dims_and_preserves_uniform_color() {
        let r = raw(8, 4, 32, 0x7F, 2.0);
        let f = bgra_to_frame(r).unwrap();
        assert_eq!(f.width(), 4);
        assert_eq!(f.height(), 2);
        assert_eq!(f.stride, 16);
        // Box-filter of constant color is the same color.
        for b in f.pixels.iter() {
            assert_eq!(*b, 0x7F);
        }
    }

    #[test]
    fn scale_2_averages_two_different_pixels() {
        // 4x2 physical, 2x1 logical. Left column = 100, right column = 200.
        let mut buf = vec![0u8; 4 * 2 * 4];
        for y in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 100 } else { 200 };
                for c in 0..4 {
                    buf[y * 16 + x * 4 + c] = v;
                }
            }
        }
        let r = RawBgra {
            width_px: 4,
            height_px: 2,
            stride: 16,
            pixels: buf,
            captured_at_ns: 0,
            scale: 2.0,
            logical_origin_x: 0,
            logical_origin_y: 0,
        };
        let f = bgra_to_frame(r).unwrap();
        // Output is 2x1. Left output pixel averages 4 physical pixels of value 100.
        // Right output pixel averages 4 physical pixels of value 200.
        assert_eq!(f.width(), 2);
        assert_eq!(f.height(), 1);
        assert_eq!(f.pixels[0], 100);
        assert_eq!(f.pixels[4], 200);
    }

    #[test]
    fn too_small_buffer_errors() {
        let mut r = raw(100, 100, 400, 0, 1.0);
        r.pixels.truncate(50);
        let e = bgra_to_frame(r).unwrap_err();
        assert_eq!(e.code(), "capture_failed");
    }

    #[test]
    fn stride_less_than_width_times_4_errors() {
        let r = raw(10, 2, 20, 0, 1.0); // stride 20 < 10*4 = 40
        let e = bgra_to_frame(r).unwrap_err();
        assert_eq!(e.code(), "capture_failed");
    }

    #[test]
    fn fractional_scale_unsupported() {
        let r = raw(10, 10, 40, 0, 1.5);
        let e = bgra_to_frame(r).unwrap_err();
        assert_eq!(e.code(), "capture_failed");
    }
}
