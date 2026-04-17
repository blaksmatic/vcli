//! Windows capture backend — v0.4 stub. Compiles so the workspace builds on
//! Windows CI, but every operation returns `Unsupported` with a clear message.

use vcli_core::frame::{Frame, FrameFormat};

use crate::capture::{Capture, WindowDescriptor};
use crate::error::CaptureError;

/// Windows capture backend stub.
#[derive(Debug, Default)]
pub struct WindowsCapture;

impl WindowsCapture {
    /// Construct.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Capture for WindowsCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        const FORMATS: &[FrameFormat] = &[FrameFormat::Bgra8];
        FORMATS
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        Err(CaptureError::Unsupported {
            what: "WindowsCapture::enumerate_windows (v0.4)",
        })
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        Err(CaptureError::Unsupported {
            what: "WindowsCapture::grab_screen (v0.4)",
        })
    }

    fn grab_window(&mut self, _window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        Err(CaptureError::Unsupported {
            what: "WindowsCapture::grab_window (v0.4)",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_reports_unsupported() {
        let w = WindowsCapture::new();
        assert_eq!(w.enumerate_windows().unwrap_err().code(), "unsupported");
    }

    #[test]
    fn grab_screen_reports_unsupported() {
        let mut w = WindowsCapture::new();
        assert_eq!(w.grab_screen().unwrap_err().code(), "unsupported");
    }

    #[test]
    fn formats_always_includes_bgra() {
        let w = WindowsCapture::new();
        assert!(w.supported_formats().contains(&FrameFormat::Bgra8));
    }
}
