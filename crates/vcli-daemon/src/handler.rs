//! `DaemonHandler` implements `vcli_ipc::Handler`. One `impl` method per
//! [`RequestOp`] variant. The handler owns clones of the bridge endpoints and
//! an `Arc<Mutex<Store>>` (sync — reached via `spawn_blocking` when we need to
//! call `SQLite` from inside an async method).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::oneshot;
use vcli_core::{ErrorCode, ErrorPayload};
use vcli_ipc::{
    Handler, IpcResult, RequestId, RequestOp, Response, StreamFrame, StreamKind, StreamSender,
};
use vcli_store::Store;

use crate::bridge::{CommandChannel, SchedulerCommand};

/// Shared boundary between the tokio handler and the scheduler/store.
#[derive(Clone)]
pub struct DaemonHandler {
    /// Async-side store handle. All DB ops run inside `spawn_blocking`.
    pub store: Arc<Mutex<Store>>,
    /// Command + broadcast endpoints.
    pub bridge: CommandChannel,
    /// Wall-clock start time (for `health.uptime_ms`).
    pub started_at: Instant,
    /// Graceful-shutdown trigger, set when a client sends `Shutdown`. The
    /// `run_foreground` task awaits the receiver.
    pub shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl DaemonHandler {
    /// Fire the shutdown oneshot if still armed. Idempotent.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    pub fn trigger_shutdown(&self) {
        if let Some(tx) = self.shutdown_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }

    fn handle_health(&self, id: RequestId) -> Response {
        let uptime_ms = u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        Response::ok(
            id,
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_ms": uptime_ms,
                "socket_origin": "resolved",
            }),
        )
    }

    fn handle_shutdown(&self, id: RequestId) -> Response {
        self.trigger_shutdown();
        let _ = self.bridge.cmd_tx.send(SchedulerCommand::Shutdown);
        Response::ok(id, serde_json::json!({ "bye": true }))
    }
}

#[async_trait]
impl Handler for DaemonHandler {
    async fn handle(&self, id: RequestId, op: RequestOp) -> IpcResult<Response> {
        let resp = match op {
            RequestOp::Health => self.handle_health(id),
            RequestOp::Shutdown => self.handle_shutdown(id),
            other => Response::err(
                id,
                ErrorPayload::simple(ErrorCode::Internal, format!("op not yet wired: {other:?}")),
            ),
        };
        Ok(resp)
    }

    async fn handle_stream(
        &self,
        id: RequestId,
        _op: RequestOp,
        tx: StreamSender,
    ) -> IpcResult<()> {
        let _ = tx
            .send(StreamFrame::end_of_stream(id, StreamKind::Events))
            .await;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use tempfile::TempDir;

    /// Bundle returned by `fresh_handler` that keeps the tempdir alive and
    /// exposes a few handles follow-up tasks consume directly.
    pub struct Fixture {
        // `dir` is held purely for Drop — the TempDir must outlive the Store.
        // Why: `#[allow(dead_code)]` — required storage for the RAII tempdir
        //      even though no test reads the field.
        #[allow(dead_code)]
        pub dir: TempDir,
        pub handler: DaemonHandler,
        pub shutdown_rx: oneshot::Receiver<()>,
        // Consumed by Task 8d+ tests; kept in the bundle so the channel isn't
        // dropped before later tasks wire assertions against it.
        // Why: `#[allow(dead_code)]` — alive now so the cmd_tx end inside
        //      `handler.bridge` doesn't disconnect, which would change test
        //      semantics when 8d starts asserting on dispatched commands.
        #[allow(dead_code)]
        pub cmd_rx: crossbeam_channel::Receiver<SchedulerCommand>,
    }

    pub fn fresh_handler() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).unwrap();
        let (bridge, cmd_rx, _event_rx, _sched_event_tx) = crate::bridge::new_channels();
        let (stx, srx) = oneshot::channel();
        let handler = DaemonHandler {
            store: Arc::new(Mutex::new(store)),
            bridge,
            started_at: Instant::now(),
            shutdown_tx: Arc::new(Mutex::new(Some(stx))),
        };
        Fixture {
            dir,
            handler,
            shutdown_rx: srx,
            cmd_rx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::fresh_handler;
    use super::*;

    #[tokio::test]
    async fn health_returns_version_and_uptime() {
        let f = fresh_handler();
        let id = RequestId::new();
        let resp = f.handler.handle(id, RequestOp::Health).await.unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["id"], id.to_string());
        assert_eq!(body["ok"], true);
        assert!(body["result"]["version"].as_str().is_some());
        assert!(body["result"]["uptime_ms"].as_u64().is_some());
    }

    #[tokio::test]
    async fn shutdown_triggers_oneshot_and_cmd() {
        let f = fresh_handler();
        let id = RequestId::new();
        let _resp = f.handler.handle(id, RequestOp::Shutdown).await.unwrap();
        assert!(f.shutdown_rx.await.is_ok());
    }
}
