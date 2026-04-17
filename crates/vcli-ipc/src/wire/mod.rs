//! Typed wire envelopes for the framed-JSON IPC protocol.
//!
//! The `Request` / `Response` / `StreamFrame` types are the only things that
//! cross `write_frame` / `read_frame` in production. Decision 2.2 governs the
//! error shape; it lives in `vcli_core::ErrorPayload` and is re-exported by
//! the response module.

pub mod request;
pub mod response;
pub mod stream;

pub use request::{Request, RequestId, RequestOp};
pub use response::{Response, ResponseBody};
pub use stream::{StreamFrame, StreamKind};
