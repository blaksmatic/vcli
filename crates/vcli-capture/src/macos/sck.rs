//! macOS ScreenCaptureKit backend — stub.

use std::sync::Mutex;

use vcli_core::frame::{Frame, FrameFormat};

use crate::capture::{Capture, WindowDescriptor};
use crate::error::CaptureError;

/// macOS SCK-backed `Capture` implementation.
pub struct MacCapture {
    _inner: Mutex<()>,
}

impl MacCapture {
    /// Construct.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if permission is denied.
    pub fn new() -> Result<Self, CaptureError> {
        Ok(Self { _inner: Mutex::new(()) })
    }
}

impl Capture for MacCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        const FORMATS: &[FrameFormat] = &[FrameFormat::Bgra8];
        FORMATS
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        Err(CaptureError::Unsupported { what: "stub" })
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        Err(CaptureError::Unsupported { what: "stub" })
    }

    fn grab_window(&mut self, _window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        Err(CaptureError::Unsupported { what: "stub" })
    }
}
