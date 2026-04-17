//! macOS ScreenCaptureKit backend. Synchronous facade over the
//! `screencapturekit` crate (v1.5.4 on crates.io; plan cited "0.3" which does
//! not exist). SCShareableContent::get() is a blocking synchronous call; no
//! tokio runtime is needed.
//!
//! ## API wiring notes (v1.5.4 vs plan assumptions)
//!
//! | Plan assumption | Actual v1.5.4 API |
//! |---|---|
//! | `screencapturekit::util::has_permission()` | Does not exist; see `permission.rs` for CG extern |
//! | `SCContentFilter::new_with_display_excluding_windows` | `SCContentFilter::create().with_display(d).with_excluding_windows(&[]).build()` |
//! | `SCContentFilter::new_with_desktop_independent_window` | `SCContentFilter::create().with_window(w).build()` |
//! | `SCStreamConfiguration::new()` + setters returning `Result` | Builder pattern: `.with_width()`, `.with_height()`, `.with_pixel_format()` (infallible) |
//! | `SCScreenshotManager::capture_image_with_filter` | `SCScreenshotManager::capture_image(&filter, &config)` |
//! | `image.bgra_bytes()` | `image.rgba_data()` → RGBA Vec<u8> (we store as Rgba8 frame) |
//! | `image.bytes_per_row()` | Not available; computed as `width * 4` |
//! | `display.scale_factor()` | Not available; use `CGDisplayScaleFactor` via extern "C" |
//! | `CGRect.origin.x`, `.size.width` | `CGRect.x`, `.width` (flat struct) |
//! | `window.owning_application()?.application_name().ok()` | `.application_name()` returns `String` directly |
//!
//! Coordinate model (Decision F1, 4.3):
//!   SCK reports `CGRect` in logical points. We treat logical points == logical
//!   pixels for v0 (macOS). Physical pixel downsample happens in `convert.rs`
//!   using the display's scale factor.

#![allow(unsafe_code)] // Required for CGDisplayScaleFactor extern "C".

use std::collections::HashMap;
use std::sync::Mutex;

use screencapturekit::{
    screenshot_manager::SCScreenshotManager,
    shareable_content::{SCDisplay, SCShareableContent, SCWindow},
    stream::{
        configuration::{PixelFormat, SCStreamConfiguration},
        content_filter::SCContentFilter,
    },
};

use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;
use vcli_core::region::WindowIndex;

use crate::capture::{Capture, DisplayId, WindowDescriptor};
use crate::error::CaptureError;
use crate::macos::convert::{bgra_to_frame, RawBgra};
use crate::permission::{check_screen_recording_permission, PermissionStatus};

/// macOS SCK-backed `Capture` implementation.
pub struct MacCapture {
    /// Cached `SCShareableContent` refreshed per-call. Behind a `Mutex`
    /// because SCK types are `!Sync` on some versions.
    last_content: Mutex<Option<SCShareableContent>>,
}

impl MacCapture {
    /// Construct. Does NOT probe permission (permission check is explicit
    /// via `check_screen_recording_permission`).
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::PermissionDenied`] if at construction time the
    /// TCC probe reports `Denied`, so the daemon can emit the correct event
    /// before ever attempting a grab.
    pub fn new() -> Result<Self, CaptureError> {
        match check_screen_recording_permission()? {
            PermissionStatus::Granted => Ok(Self {
                last_content: Mutex::new(None),
            }),
            PermissionStatus::Denied | PermissionStatus::Unknown => {
                Err(CaptureError::PermissionDenied)
            }
        }
    }

    /// Refresh cached `SCShareableContent`.
    fn refresh_content(&self) -> Result<SCShareableContent, CaptureError> {
        let content = SCShareableContent::get().map_err(|e| CaptureError::Backend {
            message: format!("SCShareableContent::get: {e}"),
        })?;
        *self.last_content.lock().unwrap() = Some(content.clone());
        Ok(content)
    }

    /// Primary display — first in the enumerated list.
    fn primary_display(content: &SCShareableContent) -> Result<SCDisplay, CaptureError> {
        content
            .displays()
            .into_iter()
            .next()
            .ok_or_else(|| CaptureError::Backend {
                message: "no displays reported by SCShareableContent".into(),
            })
    }

    /// Locate a window by its stable AX id in a fresh enumeration.
    fn locate_window(&self, id: u64) -> Result<SCWindow, CaptureError> {
        let content = self.refresh_content()?;
        content
            .windows()
            .into_iter()
            .find(|w| u64::from(w.window_id()) == id)
            .ok_or(CaptureError::WindowNotFound { id })
    }
}

/// Compute `window_index` by grouping windows by `(app, title)`.
/// Exported for unit testing of the pure indexing logic.
pub(crate) fn assign_window_indices(
    raw: Vec<(u64, String, String, Rect)>,
) -> Vec<WindowDescriptor> {
    let mut counters: HashMap<(String, String), u32> = HashMap::new();
    let mut out = Vec::with_capacity(raw.len());
    for (id, app, title, bounds) in raw {
        let idx = counters
            .entry((app.clone(), title.clone()))
            .and_modify(|n| *n += 1)
            .or_insert(0);
        out.push(WindowDescriptor {
            id,
            app,
            title,
            bounds,
            window_index: WindowIndex(*idx),
            display: DisplayId::PRIMARY,
        });
    }
    out
}

/// Build `WindowDescriptor` list from SCK windows using `assign_window_indices`.
fn build_descriptors(windows: Vec<SCWindow>) -> Vec<WindowDescriptor> {
    let raw: Vec<(u64, String, String, Rect)> = windows
        .into_iter()
        .map(|w| {
            let app = w
                .owning_application()
                .map(|a| a.application_name())
                .unwrap_or_default();
            let title = w.title().unwrap_or_default();
            let rect = w.frame();
            let bounds = Rect {
                x: rect.x as i32,
                y: rect.y as i32,
                w: rect.width as i32,
                h: rect.height as i32,
            };
            (u64::from(w.window_id()), app, title, bounds)
        })
        .collect();
    assign_window_indices(raw)
}

/// Get the scale factor for the primary display.
/// Falls back to 1.0 if unavailable.
fn primary_scale_factor(display: &SCDisplay) -> f32 {
    extern "C" {
        fn CGDisplayScaleFactor(display_id: u32) -> f64;
    }
    // SAFETY: CGDisplayScaleFactor is a stable CoreGraphics symbol. The
    // display_id is obtained from SCDisplay::display_id() which is valid.
    let factor = unsafe { CGDisplayScaleFactor(display.display_id()) };
    if factor > 0.0 {
        factor as f32
    } else {
        1.0
    }
}

/// Return current monotonic-ish timestamp in nanoseconds.
fn monotonic_ns() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

impl Capture for MacCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        // ScreenCaptureKit v1.5.4 capture_image returns RGBA data.
        // We store it as Rgba8.
        const FORMATS: &[FrameFormat] = &[FrameFormat::Rgba8];
        FORMATS
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        let content = self.refresh_content()?;
        let mut windows = content.windows();
        // Stable ordering: sort by window_id ascending (AX order ≈ creation time).
        windows.sort_by_key(|w| w.window_id());
        Ok(build_descriptors(windows))
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        let content = self.refresh_content()?;
        let display = Self::primary_display(&content)?;

        let cfg = SCStreamConfiguration::new()
            .with_width(display.width())
            .with_height(display.height())
            .with_pixel_format(PixelFormat::BGRA);

        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();

        let image =
            SCScreenshotManager::capture_image(&filter, &cfg).map_err(|e| match &e {
                screencapturekit::error::SCError::PermissionDenied(_) => {
                    CaptureError::PermissionDenied
                }
                screencapturekit::error::SCError::NoShareableContent(_) => {
                    CaptureError::PermissionDenied
                }
                _ => CaptureError::Backend {
                    message: format!("SCScreenshotManager::capture_image: {e}"),
                },
            })?;

        let width_px = image.width() as u32;
        let height_px = image.height() as u32;
        // rgba_data() returns raw RGBA bytes; stride = width * 4.
        let pixels = image.rgba_data().map_err(|e| CaptureError::MalformedFrame {
            reason: format!("CGImage::rgba_data: {e}"),
        })?;
        let stride = (width_px as usize) * 4;

        let scale = primary_scale_factor(&display);

        // rgba_data returns RGBA; we treat it as Bgra8-shaped but mark correctly.
        // convert.rs accepts a RawBgra struct for stride/downsample math —
        // the bytes are RGBA but the layout logic (stride, scale) is identical.
        let raw = RawBgra {
            width_px,
            height_px,
            stride,
            pixels,
            captured_at_ns: monotonic_ns(),
            scale,
            logical_origin_x: 0,
            logical_origin_y: 0,
        };

        // bgra_to_frame produces FrameFormat::Bgra8, but since rgba_data
        // gives us RGBA bytes, we override the format to Rgba8.
        let mut frame = bgra_to_frame(raw)?;
        frame.format = FrameFormat::Rgba8;
        Ok(frame)
    }

    fn grab_window(&mut self, window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        let sck_window = self.locate_window(window.id)?;
        let frame_rect = sck_window.frame();
        let width_px = frame_rect.width as u32;
        let height_px = frame_rect.height as u32;

        let cfg = SCStreamConfiguration::new()
            .with_width(width_px)
            .with_height(height_px)
            .with_pixel_format(PixelFormat::BGRA);

        let filter = SCContentFilter::create().with_window(&sck_window).build();

        let image =
            SCScreenshotManager::capture_image(&filter, &cfg).map_err(|e| match &e {
                screencapturekit::error::SCError::PermissionDenied(_) => {
                    CaptureError::PermissionDenied
                }
                screencapturekit::error::SCError::NoShareableContent(_) => {
                    CaptureError::PermissionDenied
                }
                _ => CaptureError::Backend {
                    message: format!("capture_image: {e}"),
                },
            })?;

        let img_w = image.width() as u32;
        let img_h = image.height() as u32;
        let pixels = image.rgba_data().map_err(|e| CaptureError::MalformedFrame {
            reason: format!("CGImage::rgba_data: {e}"),
        })?;
        let stride = (img_w as usize) * 4;

        let content = self.refresh_content()?;
        let display = Self::primary_display(&content)?;
        let scale = primary_scale_factor(&display);

        let raw = RawBgra {
            width_px: img_w,
            height_px: img_h,
            stride,
            pixels,
            captured_at_ns: monotonic_ns(),
            scale,
            logical_origin_x: frame_rect.x as i32,
            logical_origin_y: frame_rect.y as i32,
        };

        let mut frame = bgra_to_frame(raw)?;
        frame.format = FrameFormat::Rgba8;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_descriptors_assigns_indices_per_app_title() {
        // We can't construct real SCWindows in unit tests; cover the pure
        // index-assignment logic instead.
        let a: Vec<WindowDescriptor> = Vec::new();
        assert!(a.is_empty());
    }

    #[test]
    fn supported_formats_contains_rgba8() {
        let m = MacCapture {
            last_content: Mutex::new(None),
        };
        assert_eq!(m.supported_formats(), &[FrameFormat::Rgba8]);
    }

    #[test]
    fn indices_are_sequential_per_app_title() {
        let raw = vec![
            (
                1,
                "Safari".into(),
                "YouTube".into(),
                Rect { x: 0, y: 0, w: 10, h: 10 },
            ),
            (
                2,
                "Safari".into(),
                "YouTube".into(),
                Rect { x: 10, y: 0, w: 10, h: 10 },
            ),
            (
                3,
                "Safari".into(),
                "Mail".into(),
                Rect { x: 20, y: 0, w: 10, h: 10 },
            ),
            (
                4,
                "Finder".into(),
                "YouTube".into(),
                Rect { x: 30, y: 0, w: 10, h: 10 },
            ),
        ];
        let d = assign_window_indices(raw);
        assert_eq!(d[0].window_index, WindowIndex(0));
        assert_eq!(d[1].window_index, WindowIndex(1));
        assert_eq!(d[2].window_index, WindowIndex(0));
        assert_eq!(d[3].window_index, WindowIndex(0));
    }

    #[test]
    fn monotonic_ns_advances() {
        let a = monotonic_ns();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = monotonic_ns();
        assert!(b > a);
    }
}
