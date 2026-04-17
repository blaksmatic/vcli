//! Error types for the capture crate. See Task 2 for the full enum.
use thiserror::Error;

/// Placeholder — real variants added in Task 2.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Generic placeholder; replaced in Task 2.
    #[error("capture error: {0}")]
    Other(String),
}
