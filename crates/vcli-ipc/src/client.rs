//! Async client for the IPC transport. Supports:
//!   - `request(op) -> Response`                       — one-shot
//!   - `request_stream(op) -> impl Stream<StreamFrame>`— streaming
//!
//! A new `IpcClient` is one TCP-like connection; the daemon spec does not
//! require long-lived clients, so this wrapper is cheap to re-construct.

use std::path::Path;

use tokio::io::{AsyncWriteExt, BufReader, BufWriter};
use tokio::net::unix::OwnedReadHalf;
use tokio::net::UnixStream;

use crate::error::{IpcError, IpcResult};
use crate::frame::{read_frame, write_frame};
use crate::wire::request::{Request, RequestId, RequestOp};
use crate::wire::response::Response;
use crate::wire::stream::StreamFrame;

/// Connected IPC client.
pub struct IpcClient {
    stream: UnixStream,
}

impl std::fmt::Debug for IpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcClient").finish_non_exhaustive()
    }
}

impl IpcClient {
    /// Dial the server at `socket_path`. Fails immediately if the socket is
    /// absent — Decision 1.2 guarantees absence means "daemon not ready".
    ///
    /// # Errors
    /// `IpcError::Io` wrapping the underlying `ENOENT` / `ECONNREFUSED`.
    pub async fn connect(socket_path: impl AsRef<Path>) -> IpcResult<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self { stream })
    }

    /// Send a one-shot request and await its response.
    ///
    /// # Errors
    /// Transport errors bubble up as `IpcError`. Protocol-level errors arrive
    /// inside `Response::err`.
    pub async fn request(&mut self, op: RequestOp) -> IpcResult<Response> {
        let id = RequestId::new();
        let req = Request { id, op };
        let (read_half, write_half) = self.stream.split();
        let mut reader = BufReader::new(read_half);
        let mut writer = BufWriter::new(write_half);
        write_frame(&mut writer, &req).await?;
        writer.flush().await?;
        let resp: Response = read_frame(&mut reader).await?;
        if resp.id != id {
            return Err(IpcError::SocketSetup(format!(
                "response id mismatch: req={id} resp={}",
                resp.id
            )));
        }
        Ok(resp)
    }

    /// Send a streaming request. Consumes the client: the returned
    /// `ResponseStream` owns the connection and yields `StreamFrame`s until
    /// the server sends `end: true` or the connection drops.
    ///
    /// # Errors
    /// Transport failures during handshake.
    pub async fn request_stream(self, op: RequestOp) -> IpcResult<ResponseStream> {
        let id = RequestId::new();
        let req = Request { id, op };
        // Split into owned halves so that the reader can persist across frames.
        let (read_half, write_half) = self.stream.into_split();
        {
            let mut writer = BufWriter::new(write_half);
            write_frame(&mut writer, &req).await?;
            writer.flush().await?;
            // writer (and write_half) are dropped here — we only need the read side now.
        }
        Ok(ResponseStream {
            reader: BufReader::new(read_half),
            id,
            done: false,
        })
    }
}

/// Stream of `StreamFrame`s from the server. Holds the read half until drop.
///
/// The `BufReader` persists across `next_frame()` calls so that any bytes
/// pre-fetched from the OS buffer are not lost between frames.
pub struct ResponseStream {
    reader: BufReader<OwnedReadHalf>,
    id: RequestId,
    done: bool,
}

impl ResponseStream {
    /// Blocking next-frame fetch. Returns `Ok(None)` when the server has sent
    /// the terminal `end: true` frame — use this to distinguish clean end from
    /// error.
    ///
    /// # Errors
    /// Transport errors bubble up as `IpcError`.
    pub async fn next_frame(&mut self) -> IpcResult<Option<StreamFrame>> {
        if self.done {
            return Ok(None);
        }
        let frame: StreamFrame = read_frame(&mut self.reader).await?;
        if frame.id != self.id {
            return Err(IpcError::SocketSetup(format!(
                "stream frame id mismatch: req={} got={}",
                self.id, frame.id
            )));
        }
        if frame.end {
            self.done = true;
            return Ok(None);
        }
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_fails_without_server() {
        let err = IpcClient::connect("/tmp/definitely-does-not-exist.sock")
            .await
            .unwrap_err();
        assert!(matches!(err, IpcError::Io(_)));
    }
}
