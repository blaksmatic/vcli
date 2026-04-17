//! `MockCapture` — stub, filled out in Task 3.

use vcli_core::frame::{Frame, FrameFormat};

use crate::capture::{Capture, WindowDescriptor};
use crate::error::CaptureError;

/// Canned-data capture backend stub.
#[derive(Debug)]
pub struct MockCapture;

impl Capture for MockCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        const DEFAULT: &[FrameFormat] = &[FrameFormat::Bgra8];
        DEFAULT
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        Ok(Vec::new())
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        Err(CaptureError::Backend { message: "stub".into() })
    }

    fn grab_window(&mut self, _window: &WindowDescriptor) -> Result<Frame, CaptureError> {
        Err(CaptureError::Backend { message: "stub".into() })
    }
}
