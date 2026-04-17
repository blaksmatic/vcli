//! Pluggable business-logic boundary. `vcli-ipc` does not know about programs,
//! the scheduler, or `SQLite` — it just serializes calls. The daemon crate (or a
//! test double) implements this trait.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::IpcResult;
use crate::wire::request::{RequestId, RequestOp};
use crate::wire::response::Response;
use crate::wire::stream::StreamFrame;

/// A bounded sender the server hands to streaming handler calls so the handler
/// can push `StreamFrame`s back to the client. Wrapped so we can change the
/// backing channel later without breaking handler crates.
#[derive(Debug, Clone)]
pub struct StreamSender(pub mpsc::Sender<StreamFrame>);

impl StreamSender {
    /// Send one frame; yields if the channel is full (backpressure).
    ///
    /// # Errors
    /// Returns the frame back as `Err` if the client has disconnected.
    pub async fn send(&self, frame: StreamFrame) -> Result<(), StreamFrame> {
        self.0.send(frame).await.map_err(|e| e.0)
    }
}

/// Business-logic boundary. One instance serves all client connections; must be
/// `Send + Sync`. Implementations dispatch to the runtime / store / scheduler.
#[async_trait]
pub trait Handler: Send + Sync + 'static {
    /// One-shot op dispatch. Handler returns a fully-built `Response`.
    async fn handle(&self, id: RequestId, op: RequestOp) -> IpcResult<Response>;

    /// Streaming op dispatch. Handler pushes frames into `tx` until the stream
    /// ends or the client drops. `follow` semantics for `logs`/`events` live
    /// inside the handler. When the handler returns, the server sends a final
    /// `end_of_stream` frame automatically.
    async fn handle_stream(&self, id: RequestId, op: RequestOp, tx: StreamSender) -> IpcResult<()>;

    /// Classification helper: `true` if `op` should be dispatched via
    /// `handle_stream`, `false` if via `handle`. Default covers the v0 ops.
    fn is_streaming(&self, op: &RequestOp) -> bool {
        matches!(
            op,
            RequestOp::Logs { .. } | RequestOp::Events { .. } | RequestOp::Trace { .. }
        )
    }
}

/// Tiny test double used by the integration tests.
///
/// Exposed unconditionally so that integration tests (separate compilation
/// units) can import it. No prod code should use it.
pub mod test_double {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::Mutex;
    use vcli_core::{ErrorCode, ErrorPayload, Event, EventData, ProgramId};

    use super::{Handler, IpcResult, RequestId, RequestOp, Response, StreamFrame, StreamSender};

    /// In-memory handler that records every op dispatched to it.
    #[derive(Debug, Default, Clone)]
    pub struct FakeHandler {
        /// Ops received in dispatch order.
        pub received: Arc<Mutex<Vec<RequestOp>>>,
    }

    #[async_trait]
    impl Handler for FakeHandler {
        async fn handle(&self, id: RequestId, op: RequestOp) -> IpcResult<Response> {
            self.received.lock().await.push(op.clone());
            let resp = match op {
                RequestOp::Submit { .. } => Response::ok(
                    id,
                    serde_json::json!({ "program_id": ProgramId::new().to_string() }),
                ),
                RequestOp::Cancel { program_id } => Response::ok(
                    id,
                    serde_json::json!({ "program_id": program_id, "state": "cancelled" }),
                ),
                RequestOp::Status { program_id } | RequestOp::Start { program_id } => {
                    Response::ok(id, serde_json::json!({ "program_id": program_id }))
                }
                RequestOp::Resume {
                    program_id,
                    from_start,
                } => Response::ok(
                    id,
                    serde_json::json!({ "program_id": program_id, "from_start": from_start }),
                ),
                RequestOp::List { state } => Response::ok(
                    id,
                    serde_json::json!({ "state_filter": state, "items": [] }),
                ),
                RequestOp::Health => Response::ok(id, serde_json::json!({ "ok": true })),
                RequestOp::Gc => Response::ok(id, serde_json::json!({ "gc": "ok" })),
                RequestOp::Shutdown => Response::ok(id, serde_json::json!({ "bye": true })),
                RequestOp::Logs { .. } | RequestOp::Events { .. } | RequestOp::Trace { .. } => {
                    // Should not arrive on the one-shot path.
                    Response::err(
                        id,
                        ErrorPayload::simple(ErrorCode::Internal, "streaming op on one-shot"),
                    )
                }
            };
            Ok(resp)
        }

        async fn handle_stream(
            &self,
            id: RequestId,
            op: RequestOp,
            tx: StreamSender,
        ) -> IpcResult<()> {
            self.received.lock().await.push(op.clone());
            let two = [
                StreamFrame::event(
                    id,
                    Event {
                        at: 1,
                        data: EventData::DaemonStarted {
                            version: "0.0.1".into(),
                        },
                    },
                ),
                StreamFrame::event(
                    id,
                    Event {
                        at: 2,
                        data: EventData::DaemonStopped,
                    },
                ),
            ];
            for f in two {
                if tx.send(f).await.is_err() {
                    break;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use vcli_core::ProgramId;

    use super::{Handler, IpcResult, RequestId, RequestOp, Response, StreamSender};

    #[test]
    fn is_streaming_classifies_v0_ops() {
        struct H;
        #[async_trait]
        impl Handler for H {
            async fn handle(&self, id: RequestId, _op: RequestOp) -> IpcResult<Response> {
                Ok(Response::ok(id, serde_json::Value::Null))
            }
            async fn handle_stream(
                &self,
                _id: RequestId,
                _op: RequestOp,
                _tx: StreamSender,
            ) -> IpcResult<()> {
                Ok(())
            }
        }
        let h = H;
        assert!(h.is_streaming(&RequestOp::Events { follow: true }));
        assert!(h.is_streaming(&RequestOp::Trace {
            program_id: ProgramId::new()
        }));
        assert!(h.is_streaming(&RequestOp::Logs {
            program_id: ProgramId::new(),
            follow: false
        }));
        assert!(!h.is_streaming(&RequestOp::Health));
        assert!(!h.is_streaming(&RequestOp::List { state: None }));
    }
}
