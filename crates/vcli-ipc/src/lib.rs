//! vcli-ipc — framed-JSON IPC over Unix domain sockets.
//!
//! See the spec §IPC (`docs/superpowers/specs/2026-04-16-vcli-design.md`) for
//! the authoritative wire contract. Decisions 1.2, 1.7, 2.2, and 2.5 in that
//! file's appendix govern readiness, streaming, error shape, and socket path.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod error;

pub use error::{IpcError, IpcResult};

pub mod socket_path;

pub use socket_path::{default_socket_path, SocketPath};

pub mod frame;

pub use frame::{read_frame, write_frame, MAX_FRAME_LEN};

pub mod wire;

pub use wire::{
    Request, RequestId, RequestOp, Response, ResponseBody, StreamFrame, StreamKind,
};

pub mod handler;

pub use handler::{Handler, StreamSender};

pub mod server;

pub use server::IpcServer;
