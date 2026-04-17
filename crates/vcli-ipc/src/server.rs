//! Server accept loop. One `Handler` shared across all connections; per-
//! connection state lives in spawned tokio tasks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::AsyncWrite;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use vcli_core::{ErrorCode, ErrorPayload};

use crate::error::{IpcError, IpcResult};
use crate::frame::{read_frame, write_frame};
use crate::handler::{Handler, StreamSender};
use crate::wire::request::Request;
use crate::wire::response::Response;
use crate::wire::stream::{StreamFrame, StreamKind};

/// Bound Unix-socket IPC server. Holds onto the socket path so `Drop` can unlink.
pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
    handler: Arc<dyn Handler>,
}

impl std::fmt::Debug for IpcServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcServer")
            .field("socket_path", &self.socket_path)
            .finish_non_exhaustive()
    }
}

impl IpcServer {
    /// Bind at `socket_path`. Removes a stale socket file if it exists.
    /// Sets mode `0o600`. Matches Decision 1.2: daemon calls `bind` last.
    ///
    /// # Errors
    /// Returns `IpcError::SocketSetup` if bind or `chmod` fails.
    pub fn bind(socket_path: impl AsRef<Path>, handler: Arc<dyn Handler>) -> IpcResult<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)
                .map_err(|e| IpcError::SocketSetup(format!("unlink stale socket: {e}")))?;
        }
        let listener = UnixListener::bind(&socket_path)
            .map_err(|e| IpcError::SocketSetup(format!("bind {}: {e}", socket_path.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&socket_path, perms)
                .map_err(|e| IpcError::SocketSetup(format!("chmod 0600: {e}")))?;
        }
        Ok(Self {
            listener,
            socket_path,
            handler,
        })
    }

    /// The bound path, for display in `vcli health` and logs.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Run the accept loop until cancelled. Each accepted connection is
    /// spawned as an independent task; failures on one connection do not
    /// affect others. Returns `Ok(())` only if `shutdown_signal` completes.
    ///
    /// # Errors
    /// Returns `IpcError::Io` if `accept()` fails with a non-recoverable error.
    pub async fn serve(
        self,
        mut shutdown_signal: tokio::sync::oneshot::Receiver<()>,
    ) -> IpcResult<()> {
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown_signal => return Ok(()),
                accept = self.listener.accept() => {
                    let (stream, _addr) = accept?;
                    let handler = self.handler.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, handler).await {
                            // Connection-scoped errors never poison the server.
                            tracing_hack::log_conn_err(&e);
                        }
                    });
                }
            }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        // Best-effort unlink; ignore errors during shutdown.
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Per-connection loop. Reads one framed `Request`, dispatches, writes frames
/// back, then loops. Exits on clean EOF (peer closed between frames) or the
/// first hard error (invalid JSON, oversize frame, mid-frame EOF).
pub(crate) async fn handle_connection(
    mut stream: UnixStream,
    handler: Arc<dyn Handler>,
) -> IpcResult<()> {
    let (read_half, write_half) = stream.split();
    let mut reader = tokio::io::BufReader::new(read_half);
    let mut writer = tokio::io::BufWriter::new(write_half);

    loop {
        let req: Request = match read_frame(&mut reader).await {
            Ok(r) => r,
            Err(IpcError::UnexpectedEof {
                got: 0,
                expected: 4,
            }) => return Ok(()),
            Err(e @ (IpcError::InvalidJson(_) | IpcError::FrameTooLarge { .. })) => {
                // Best-effort: try to report the error with a synthesized id.
                let resp = Response::err(
                    crate::wire::request::RequestId::new(),
                    ErrorPayload::simple(ErrorCode::Internal, format!("{e}")),
                );
                let _ = write_frame(&mut writer, &resp).await;
                return Err(e);
            }
            Err(e) => return Err(e),
        };

        if handler.is_streaming(&req.op) {
            dispatch_stream(&mut writer, handler.clone(), req).await?;
        } else {
            dispatch_oneshot(&mut writer, handler.clone(), req).await?;
        }
    }
}

async fn dispatch_oneshot<W>(
    writer: &mut W,
    handler: Arc<dyn Handler>,
    req: Request,
) -> IpcResult<()>
where
    W: AsyncWrite + Unpin,
{
    let Request { id, op } = req;
    let resp = match handler.handle(id, op).await {
        Ok(r) => r,
        Err(e) => Response::err(
            id,
            ErrorPayload::simple(ErrorCode::Internal, format!("{e}")),
        ),
    };
    write_frame(writer, &resp).await
}

async fn dispatch_stream<W>(
    writer: &mut W,
    handler: Arc<dyn Handler>,
    req: Request,
) -> IpcResult<()>
where
    W: AsyncWrite + Unpin,
{
    let Request { id, op } = req;
    let stream_kind = match op {
        RequestOp::Trace { .. } => StreamKind::Trace,
        _ => StreamKind::Events,
    };
    let (tx, mut rx) = mpsc::channel::<StreamFrame>(64);
    let sender = StreamSender(tx);
    let op_for_task = op.clone();
    let handler_task = handler.clone();
    let handler_fut =
        tokio::spawn(async move { handler_task.handle_stream(id, op_for_task, sender).await });

    while let Some(frame) = rx.recv().await {
        write_frame(writer, &frame).await?;
    }

    // Flush terminal frame — handler task has finished and channel has drained.
    let _ = handler_fut.await;
    let end = StreamFrame::end_of_stream(id, stream_kind);
    write_frame(writer, &end).await
}

use crate::wire::request::RequestOp;

// Tiny inline shim so we don't pull in the real `tracing` crate as a dep.
// Daemon can enable proper tracing when it wires things up.
mod tracing_hack {
    use crate::error::IpcError;
    pub fn log_conn_err(_e: &IpcError) {
        // intentionally empty — the daemon will wire real logging later.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::test_double::FakeHandler;
    use tempfile::TempDir;

    fn sock_path(dir: &TempDir) -> PathBuf {
        dir.path().join("vcli.sock")
    }

    #[tokio::test]
    async fn bind_fails_when_parent_missing() {
        let handler: Arc<dyn Handler> = Arc::new(FakeHandler::default());
        let err = IpcServer::bind("/no/such/dir/vcli.sock", handler).unwrap_err();
        assert!(matches!(err, IpcError::SocketSetup(_)));
    }

    #[tokio::test]
    async fn bind_removes_stale_socket_file() {
        let dir = TempDir::new().unwrap();
        let p = sock_path(&dir);
        std::fs::write(&p, b"stale").unwrap();
        let handler: Arc<dyn Handler> = Arc::new(FakeHandler::default());
        let server = IpcServer::bind(&p, handler).unwrap();
        assert_eq!(server.socket_path(), p);
    }

    #[tokio::test]
    async fn drop_unlinks_socket() {
        let dir = TempDir::new().unwrap();
        let p = sock_path(&dir);
        let handler: Arc<dyn Handler> = Arc::new(FakeHandler::default());
        {
            let server = IpcServer::bind(&p, handler).unwrap();
            assert!(p.exists());
            drop(server);
        }
        assert!(!p.exists());
    }
}
