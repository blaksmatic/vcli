//! Recording InputSink: captures every method call into a `Vec<Call>` for assertion.

use std::sync::Mutex;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;
use vcli_input::error::InputError;
use vcli_input::sink::{DragSegment, InputSink};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Move(Point),
    Click(Point, Button),
    DoubleClick(Point, Button),
    Drag(Point, Vec<DragSegment>, Button),
    Type(String),
    Key(Vec<Modifier>, String),
}

#[derive(Default)]
pub struct RecordingInputSink {
    pub calls: Mutex<Vec<Call>>,
}

impl RecordingInputSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl InputSink for RecordingInputSink {
    fn mouse_move(&self, to: Point) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::Move(to));
        Ok(())
    }

    fn click(
        &self,
        at: Point,
        button: Button,
        _: &[Modifier],
        _: u32,
    ) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::Click(at, button));
        Ok(())
    }

    fn double_click(&self, at: Point, button: Button) -> Result<(), InputError> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::DoubleClick(at, button));
        Ok(())
    }

    fn drag(
        &self,
        from: Point,
        segs: &[DragSegment],
        button: Button,
    ) -> Result<(), InputError> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Drag(from, segs.to_vec(), button));
        Ok(())
    }

    fn type_text(&self, s: &str) -> Result<(), InputError> {
        self.calls.lock().unwrap().push(Call::Type(s.to_string()));
        Ok(())
    }

    fn key_combo(&self, mods: &[Modifier], k: &str) -> Result<(), InputError> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Key(mods.to_vec(), k.to_string()));
        Ok(())
    }
}
