//! `MockCapture` — canned frames + windows for deterministic tests.
//!
//! Used by vcli-runtime scenario harness, vcli-perception evaluator tests,
//! and the vcli-daemon ipc handler in its test suite.

use std::sync::{Arc, Mutex};

use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;
use vcli_core::region::WindowIndex;

use crate::capture::{Capture, DisplayId, WindowDescriptor};
use crate::error::CaptureError;

/// Canned-data capture backend. Cycles through `screen_frames` on each
/// `grab_screen()`. `window_frames` lookup by window id. `enumerate_windows`
/// returns a clone of the configured list. Thread-safe via `Mutex`.
#[derive(Debug)]
pub struct MockCapture {
    inner: Arc<Mutex<MockInner>>,
}

#[derive(Debug)]
struct MockInner {
    #[allow(dead_code)] // stored for future multi-format support
    formats: Vec<FrameFormat>,
    windows: Vec<WindowDescriptor>,
    screen_frames: Vec<Frame>,
    screen_cursor: usize,
    window_frame_map: Vec<(u64, Vec<Frame>)>,
    window_cursors: Vec<usize>,
    next_error: Option<CaptureError>,
}

impl MockCapture {
    /// Empty mock — all calls return default / empty. For the "no programs,
    /// nothing to capture" path.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(vec![FrameFormat::Bgra8], vec![], vec![])
    }

    /// Full constructor.
    #[must_use]
    pub fn new(
        formats: Vec<FrameFormat>,
        windows: Vec<WindowDescriptor>,
        screen_frames: Vec<Frame>,
    ) -> Self {
        let window_cursors = vec![0; windows.len()];
        Self {
            inner: Arc::new(Mutex::new(MockInner {
                formats,
                windows,
                screen_frames,
                screen_cursor: 0,
                window_frame_map: Vec::new(),
                window_cursors,
                next_error: None,
            })),
        }
    }

    /// Set canned frames for a particular window id. Subsequent calls to
    /// `grab_window` with that descriptor cycle through these frames.
    pub fn set_window_frames(&self, window_id: u64, frames: Vec<Frame>) {
        let mut g = self.inner.lock().unwrap();
        if let Some((_, slot)) =
            g.window_frame_map.iter_mut().find(|(id, _)| *id == window_id)
        {
            *slot = frames;
        } else {
            g.window_frame_map.push((window_id, frames));
        }
    }

    /// Arm the next call to any grab/enumerate method to fail with `e`. Used
    /// to exercise error paths in consumer tests. Consumed by one failing call.
    pub fn arm_error(&self, e: CaptureError) {
        self.inner.lock().unwrap().next_error = Some(e);
    }

    fn take_armed_error(inner: &mut MockInner) -> Option<CaptureError> {
        inner.next_error.take()
    }
}

impl Capture for MockCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        // Safety-ish: we hand out a leaked static slice derived from the
        // current config by cloning into a boxed leak only once. To keep this
        // simple and allocation-free on the hot path, we return a constant
        // slice with the default format.
        const DEFAULT: &[FrameFormat] = &[FrameFormat::Bgra8];
        DEFAULT
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(e) = Self::take_armed_error(&mut g) {
            return Err(e);
        }
        Ok(g.windows.clone())
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(e) = Self::take_armed_error(&mut g) {
            return Err(e);
        }
        if g.screen_frames.is_empty() {
            return Err(CaptureError::Backend {
                message: "MockCapture has no screen frames configured".into(),
            });
        }
        let idx = g.screen_cursor % g.screen_frames.len();
        g.screen_cursor = g.screen_cursor.wrapping_add(1);
        Ok(g.screen_frames[idx].clone())
    }

    fn grab_window(&mut self, window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(e) = Self::take_armed_error(&mut g) {
            return Err(e);
        }
        let pos = g.window_frame_map.iter().position(|(id, _)| *id == window.id);
        let Some(pos) = pos else {
            return Err(CaptureError::WindowNotFound { id: window.id });
        };
        let frames_len = g.window_frame_map[pos].1.len();
        if frames_len == 0 {
            return Err(CaptureError::Backend {
                message: format!("no frames for window id={}", window.id),
            });
        }
        while g.window_cursors.len() <= pos {
            g.window_cursors.push(0);
        }
        let cursor = g.window_cursors[pos];
        let idx = cursor % frames_len;
        g.window_cursors[pos] = cursor.wrapping_add(1);
        Ok(g.window_frame_map[pos].1[idx].clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn tiny(format: FrameFormat, value: u8) -> Frame {
        let bytes: Arc<[u8]> = vec![value; 4 * 4 * 2].into();
        Frame::new(format, Rect { x: 0, y: 0, w: 4, h: 2 }, 4 * 4, bytes, 0)
    }

    #[test]
    fn empty_mock_returns_empty_windows() {
        let m = MockCapture::empty();
        assert!(m.enumerate_windows().unwrap().is_empty());
    }

    #[test]
    fn empty_mock_screen_grab_errors() {
        let mut m = MockCapture::empty();
        let e = m.grab_screen().unwrap_err();
        assert_eq!(e.code(), "capture_failed");
    }

    #[test]
    fn screen_frames_cycle_in_order() {
        let frames = vec![tiny(FrameFormat::Bgra8, 1), tiny(FrameFormat::Bgra8, 2)];
        let mut m = MockCapture::new(vec![FrameFormat::Bgra8], vec![], frames);
        let a = m.grab_screen().unwrap();
        let b = m.grab_screen().unwrap();
        let c = m.grab_screen().unwrap();
        assert_eq!(a.pixels[0], 1);
        assert_eq!(b.pixels[0], 2);
        assert_eq!(c.pixels[0], 1); // wraps
    }

    #[test]
    fn enumerate_returns_configured_windows() {
        let w = WindowDescriptor {
            id: 9,
            app: "Finder".into(),
            title: "Downloads".into(),
            bounds: Rect { x: 0, y: 0, w: 1000, h: 600 },
            window_index: WindowIndex(0),
            display: DisplayId::PRIMARY,
        };
        let m = MockCapture::new(vec![FrameFormat::Bgra8], vec![w.clone()], vec![]);
        let got = m.enumerate_windows().unwrap();
        assert_eq!(got, vec![w]);
    }

    #[test]
    fn grab_window_uses_configured_frames() {
        let w = WindowDescriptor {
            id: 7,
            app: "Safari".into(),
            title: "YT".into(),
            bounds: Rect { x: 0, y: 0, w: 4, h: 2 },
            window_index: WindowIndex(0),
            display: DisplayId::PRIMARY,
        };
        let m = MockCapture::new(vec![FrameFormat::Bgra8], vec![w.clone()], vec![]);
        m.set_window_frames(7, vec![tiny(FrameFormat::Bgra8, 42)]);
        let mut m2 = m;
        let f = m2.grab_window(&w).unwrap();
        assert_eq!(f.pixels[0], 42);
    }

    #[test]
    fn grab_window_unknown_id_errors() {
        let w = WindowDescriptor {
            id: 100,
            app: "X".into(),
            title: "Y".into(),
            bounds: Rect { x: 0, y: 0, w: 4, h: 2 },
            window_index: WindowIndex(0),
            display: DisplayId::PRIMARY,
        };
        let mut m = MockCapture::new(vec![FrameFormat::Bgra8], vec![], vec![]);
        let e = m.grab_window(&w).unwrap_err();
        assert_eq!(e.code(), "unknown_window");
    }

    #[test]
    fn armed_error_is_returned_once() {
        let mut m = MockCapture::new(
            vec![FrameFormat::Bgra8],
            vec![],
            vec![tiny(FrameFormat::Bgra8, 0)],
        );
        m.arm_error(CaptureError::PermissionDenied);
        assert_eq!(m.grab_screen().unwrap_err().code(), "permission_denied");
        // Second call succeeds because error was consumed.
        assert!(m.grab_screen().is_ok());
    }

    #[test]
    fn mock_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockCapture>();
    }

    #[test]
    fn mock_works_through_dyn_capture() {
        use crate::capture::Capture;
        let mut m = MockCapture::new(
            vec![FrameFormat::Bgra8],
            vec![],
            vec![tiny(FrameFormat::Bgra8, 9)],
        );
        let dyn_cap: &mut dyn Capture = &mut m;
        let f = dyn_cap.grab_screen().unwrap();
        assert_eq!(f.pixels[0], 9);
    }
}
