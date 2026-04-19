//! Scripted Capture: returns a queue of pre-built Frames, in order, then
//! repeats the last one. `enumerate_windows` returns an empty list.

use std::sync::{Arc, Mutex};

use vcli_capture::capture::{Capture, WindowDescriptor};
use vcli_capture::error::CaptureError;
use vcli_core::frame::{Frame, FrameFormat};
use vcli_core::geom::Rect;

pub struct ScriptedCapture {
    frames: Mutex<Vec<Frame>>,
    cursor: Mutex<usize>,
}

impl ScriptedCapture {
    #[must_use]
    pub fn new(frames: Vec<Frame>) -> Self {
        Self {
            frames: Mutex::new(frames),
            cursor: Mutex::new(0),
        }
    }

    #[must_use]
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
        let n = (width * height) as usize;
        let mut buf = Vec::with_capacity(n * 4);
        for _ in 0..n {
            buf.extend_from_slice(&rgba);
        }
        Frame::new(
            FrameFormat::Rgba8,
            Rect {
                x: 0,
                y: 0,
                w: i32::try_from(width).unwrap(),
                h: i32::try_from(height).unwrap(),
            },
            4,
            Arc::from(buf),
            0,
        )
    }
}

impl Capture for ScriptedCapture {
    fn supported_formats(&self) -> &[FrameFormat] {
        &[FrameFormat::Rgba8]
    }

    fn enumerate_windows(&self) -> Result<Vec<WindowDescriptor>, CaptureError> {
        Ok(vec![])
    }

    fn grab_screen(&mut self) -> Result<Frame, CaptureError> {
        let frames = self.frames.lock().unwrap();
        let mut cursor = self.cursor.lock().unwrap();
        if frames.is_empty() {
            return Ok(Self::solid(1, 1, [0, 0, 0, 0]));
        }
        let i = (*cursor).min(frames.len() - 1);
        let out = frames[i].clone();
        *cursor = (i + 1).min(frames.len() - 1);
        Ok(out)
    }

    fn grab_window(&mut self, _: &WindowDescriptor) -> Result<Frame, CaptureError> {
        self.grab_screen()
    }
}
