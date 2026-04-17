//! Windows stub backend. v0.4 replaces this with a real implementation (see
//! spec §Roadmap). Currently all methods return `InputError::Unimplemented`.

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use crate::error::InputError;
use crate::sink::{DragSegment, InputSink};

/// Windows stub sink. Constructible but every method returns `Unimplemented`.
#[derive(Debug, Default)]
pub struct WindowsInputSink;

impl WindowsInputSink {
    /// Constructor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl InputSink for WindowsInputSink {
    fn mouse_move(&self, _to: Point) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn click(&self, _: Point, _: Button, _: &[Modifier], _: u32) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn double_click(&self, _: Point, _: Button) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn drag(&self, _: Point, _: &[DragSegment], _: Button) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn type_text(&self, _: &str) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
    fn key_combo(&self, _: &[Modifier], _: &str) -> Result<(), InputError> {
        unimplemented!("vcli-input Windows backend — see roadmap v0.4")
    }
}
