//! `Capture` trait and associated descriptor types.

use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;
use vcli_core::region::WindowIndex;

use crate::error::CaptureError;

/// Opaque identifier for a display. v0 is single-display (primary) so this is
/// effectively `DisplayId::PRIMARY`, but the type is here from day 1 to keep
/// the trait stable when multi-display lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayId(pub u32);

impl DisplayId {
    /// The primary display. Backends may enumerate others in the future.
    pub const PRIMARY: Self = Self(0);
}

/// A window known to the capture backend. Stable for the lifetime of the
/// window; ordering matches the native enumeration order (macOS: oldest →
/// newest per Decision F2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDescriptor {
    /// Backend-assigned stable id. For macOS this is the AX window id (u32)
    /// re-exposed. Opaque to callers.
    pub id: u64,
    /// App/owner name, e.g. "Safari".
    pub app: String,
    /// Window title string, possibly empty.
    pub title: String,
    /// Bounds in logical pixels, relative to the display origin.
    pub bounds: Rect,
    /// 0-based index within the (app, title-substring) group this window
    /// belongs to in the current enumeration pass. The region resolver in
    /// the perception lane uses this directly as `WindowIndex`.
    pub window_index: WindowIndex,
    /// Display the window is currently on.
    pub display: DisplayId,
}

/// Capture backend. One capture per tick; frame is shared across programs.
pub trait Capture: Send + Sync {
    /// Pixel formats this backend can emit. First entry is the preferred one.
    fn supported_formats(&self) -> &[FrameFormat];

    /// Enumerate visible application windows on all displays. Excludes
    /// off-screen and minimized windows. Stable ordering per backend.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if the backend fails to enumerate windows.
    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError>;

    /// Grab a full-screen frame of the primary display.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if the backend fails to capture the screen.
    fn grab_screen(&mut self) -> Result<Frame, CaptureError>;

    /// Grab a frame cropped to the given window's current bounds. Backend
    /// may re-resolve the window by id; if the window has moved/resized
    /// between enumeration and grab, the returned `Frame.bounds` reflects
    /// the actual capture, not the stale descriptor bounds.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if the window is not found or capture fails.
    fn grab_window(&mut self, window: &WindowDescriptor) -> Result<Frame, CaptureError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_id_primary_is_zero() {
        assert_eq!(DisplayId::PRIMARY, DisplayId(0));
    }

    #[test]
    fn window_descriptor_is_clone_eq() {
        let a = WindowDescriptor {
            id: 42,
            app: "Safari".into(),
            title: "YouTube".into(),
            bounds: Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            window_index: WindowIndex(0),
            display: DisplayId::PRIMARY,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn capture_trait_is_object_safe() {
        // If this compiles, `dyn Capture` works — required for scheduler injection.
        fn _takes_dyn(_c: &mut dyn Capture) {}
    }
}
