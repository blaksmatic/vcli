//! Transport-level IPC errors. Protocol-level errors (bad program, unknown id)
//! travel inside `Response::Err(ErrorPayload)` using `vcli_core::ErrorCode`.

use std::io;

use thiserror::Error;

/// Transport-layer errors raised by frame codec, server accept loop, client dial.
#[derive(Debug, Error)]
pub enum IpcError {
    /// Underlying I/O failure.
    #[error("io: {0}")]
    Io(#[from] io::Error),

    /// Peer closed the connection before a full frame was read.
    #[error("peer disconnected mid-frame (got {got} of {expected} bytes)")]
    UnexpectedEof {
        /// Bytes received before EOF.
        got: usize,
        /// Bytes expected for this frame.
        expected: usize,
    },

    /// Frame length header exceeds `MAX_FRAME_LEN`.
    #[error("frame too large: {len} bytes (max {max})")]
    FrameTooLarge {
        /// Declared frame length.
        len: u32,
        /// Configured maximum.
        max: u32,
    },

    /// Frame body is not valid UTF-8 JSON.
    #[error("invalid json frame: {0}")]
    InvalidJson(#[from] serde_json::Error),

    /// Server could not bind or remove stale socket.
    #[error("socket setup: {0}")]
    SocketSetup(String),

    /// No home / TMPDIR / runtime dir discoverable for socket path.
    #[error("could not resolve socket path: {0}")]
    SocketPath(String),

    /// Client called `recv` after server closed stream.
    #[error("stream closed by server")]
    StreamClosed,
}

/// Convenience alias for results at the IPC transport layer.
pub type IpcResult<T> = Result<T, IpcError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_io_error() {
        let io = io::Error::new(io::ErrorKind::ConnectionReset, "peer reset");
        let e: IpcError = io.into();
        assert!(matches!(e, IpcError::Io(_)));
    }

    #[test]
    fn from_json_error() {
        let bad: Result<serde_json::Value, _> = serde_json::from_str("{not json");
        let e: IpcError = bad.unwrap_err().into();
        assert!(matches!(e, IpcError::InvalidJson(_)));
    }

    #[test]
    fn display_includes_context() {
        let e = IpcError::FrameTooLarge { len: 99, max: 10 };
        let s = e.to_string();
        assert!(s.contains("99"), "{s}");
        assert!(s.contains("10"), "{s}");
    }

    #[test]
    fn unexpected_eof_reports_progress() {
        let e = IpcError::UnexpectedEof { got: 3, expected: 8 };
        let s = e.to_string();
        assert!(s.contains("3"));
        assert!(s.contains("8"));
    }
}
