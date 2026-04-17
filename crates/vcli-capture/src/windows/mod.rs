//! Windows capture backend — v0.4 stub.

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
        Err(CaptureError::Other("stub".into()))
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        Err(CaptureError::Other("stub".into()))
    }

    fn grab_window(&mut self, _window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        Err(CaptureError::Other("stub".into()))
    }
}
