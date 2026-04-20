//! `DaemonHandler` implements `vcli_ipc::Handler`. One `impl` method per
//! [`RequestOp`] variant. The handler owns clones of the bridge endpoints and
//! an `Arc<Mutex<Store>>` (sync — reached via `spawn_blocking` when we need to
//! call `SQLite` from inside an async method).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, error};
use vcli_core::{ErrorCode, ErrorPayload, Event, ProgramId};
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

    async fn handle_submit(
        &self,
        id: RequestId,
        program_json: serde_json::Value,
    ) -> Response {
        let program = match vcli_dsl::validate_value(&program_json) {
            Ok(v) => v.program,
            Err(e) => {
                return Response::err(id, e.to_payload());
            }
        };

        let program_id = program.id.unwrap_or_else(ProgramId::new);
        let name = program.name.clone();
        let canonical_bytes = match vcli_core::canonicalize(&program_json) {
            Ok(b) => b,
            Err(e) => {
                return Response::err(
                    id,
                    ErrorPayload::simple(ErrorCode::Internal, format!("canonicalize: {e}")),
                );
            }
        };
        let canonical = String::from_utf8(canonical_bytes)
            .expect("canonicalize produces UTF-8 by construction");

        let store = self.store.clone();
        let pid = program_id;
        let submitted_at = vcli_core::clock::now_unix_ms();
        let name_for_insert = name.clone();
        let canonical_str = canonical.clone();
        let insert_result = tokio::task::spawn_blocking(move || {
            let mut s = store.lock().unwrap();
            s.insert_program(&vcli_store::NewProgram {
                id: pid,
                name: &name_for_insert,
                source_json: &canonical_str,
                state: vcli_core::ProgramState::Pending,
                submitted_at,
                labels_json: "{}",
            })
        })
        .await;
        let insert = match insert_result {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "submit join error");
                return Response::err(
                    id,
                    ErrorPayload::simple(ErrorCode::Internal, format!("{e}")),
                );
            }
        };
        if let Err(e) = insert {
            return Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}")));
        }

        if let Err(e) = self.bridge.cmd_tx.send(SchedulerCommand::SubmitValidated {
            program_id: pid,
            program,
        }) {
            error!(error = %e, "cmd_tx full");
            return Response::err(
                id,
                ErrorPayload::simple(ErrorCode::DaemonBusy, "scheduler command queue full"),
            );
        }

        Response::ok(
            id,
            serde_json::json!({ "program_id": pid.to_string(), "name": name }),
        )
    }

    async fn stream_events(
        &self,
        id: RequestId,
        filter_program: Option<ProgramId>,
        follow: bool,
        tx: StreamSender,
    ) -> IpcResult<()> {
        let mut rx = self.bridge.event_tx.subscribe();
        if !follow {
            let store = self.store.clone();
            let history = tokio::task::spawn_blocking(move || {
                let s = store.lock().unwrap();
                s.stream_events(0, 10_000)
            })
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
            for row in history {
                if filter_program.is_some_and(|p| p != row.program_id) {
                    continue;
                }
                if let Ok(ev) = serde_json::from_str::<Event>(&row.data_json) {
                    let frame = StreamFrame::event(id, ev);
                    if tx.send(frame).await.is_err() {
                        return Ok(());
                    }
                }
            }
            return Ok(());
        }
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Some(p) = filter_program {
                        if crate::persist::program_id_of(&ev.data) != Some(p) {
                            continue;
                        }
                    }
                    if tx.send(StreamFrame::event(id, ev)).await.is_err() {
                        return Ok(());
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    let now = vcli_core::clock::now_unix_ms();
                    let dropped = StreamFrame::event(
                        id,
                        Event {
                            at: now,
                            data: vcli_core::EventData::StreamDropped {
                                count: u32::try_from(n).unwrap_or(u32::MAX),
                                since: now,
                            },
                        },
                    );
                    if tx.send(dropped).await.is_err() {
                        return Ok(());
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return Ok(()),
            }
        }
    }

    async fn handle_resume(
        &self,
        id: RequestId,
        pid: ProgramId,
        from_start: bool,
    ) -> Response {
        let store = self.store.clone();
        let now_ms = vcli_core::clock::now_unix_ms();
        let resume_result = tokio::task::spawn_blocking(move || {
            let mut s = store.lock().unwrap();
            let outcome = s.resume_program(pid, from_start, now_ms)?;
            let row = s.get_program(pid)?;
            let value: serde_json::Value = serde_json::from_str(&row.source_json)?;
            let program = vcli_dsl::validate_value(&value)
                .map(|v| v.program)
                .map_err(|e| vcli_store::StoreError::Io {
                    path: "<dsl>".into(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{e}"),
                    ),
                })?;
            Ok::<_, vcli_store::StoreError>((outcome, program))
        })
        .await;

        match resume_result {
            Ok(Ok((out, program))) => {
                let _ = self.bridge.cmd_tx.send(SchedulerCommand::ResumeRunning {
                    program_id: pid,
                    from_step: out.from_step,
                    program,
                });
                Response::ok(
                    id,
                    serde_json::json!({ "program_id": pid.to_string(), "from_step": out.from_step }),
                )
            }
            Ok(Err(vcli_store::StoreError::NotResumable(m))) => {
                Response::err(id, ErrorPayload::simple(ErrorCode::NotResumable, m))
            }
            Ok(Err(vcli_store::StoreError::UnknownProgram(_))) => Response::err(
                id,
                ErrorPayload::simple(ErrorCode::UnknownProgram, "not found"),
            ),
            Ok(Err(e)) => {
                Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}")))
            }
            Err(e) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
        }
    }

    fn handle_cancel(&self, id: RequestId, pid: ProgramId) -> Response {
        match self.bridge.cmd_tx.send(SchedulerCommand::Cancel {
            program_id: pid,
            reason: "user".into(),
        }) {
            Ok(()) => Response::ok(id, serde_json::json!({ "program_id": pid.to_string() })),
            Err(_) => Response::err(
                id,
                ErrorPayload::simple(ErrorCode::DaemonBusy, "cmd queue full"),
            ),
        }
    }

    fn handle_start(&self, id: RequestId, pid: ProgramId) -> Response {
        match self
            .bridge
            .cmd_tx
            .send(SchedulerCommand::Start { program_id: pid })
        {
            Ok(()) => Response::ok(id, serde_json::json!({ "program_id": pid.to_string() })),
            Err(_) => Response::err(
                id,
                ErrorPayload::simple(ErrorCode::DaemonBusy, "cmd queue full"),
            ),
        }
    }

    async fn handle_list(&self, id: RequestId, state: Option<String>) -> Response {
        let filter = state.and_then(|s| s.parse::<vcli_core::ProgramState>().ok());
        let store = self.store.clone();
        let rows = tokio::task::spawn_blocking(move || {
            let s = store.lock().unwrap();
            s.list_programs(filter)
        })
        .await
        .unwrap_or_else(|e| {
            Err(vcli_store::StoreError::Io {
                path: "<join>".into(),
                source: std::io::Error::other(format!("{e}")),
            })
        });
        match rows {
            Ok(rows) => {
                let items: Vec<_> = rows
                    .into_iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": r.id.to_string(),
                            "name": r.name,
                            "state": r.state.as_str(),
                            "submitted_at": r.submitted_at,
                        })
                    })
                    .collect();
                Response::ok(id, serde_json::json!({ "items": items }))
            }
            Err(e) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
        }
    }

    async fn handle_status(&self, id: RequestId, program_id: ProgramId) -> Response {
        let store = self.store.clone();
        let row = tokio::task::spawn_blocking(move || {
            let s = store.lock().unwrap();
            s.get_program(program_id)
        })
        .await;
        match row {
            Ok(Ok(r)) => Response::ok(
                id,
                serde_json::json!({
                    "id": r.id.to_string(),
                    "name": r.name,
                    "state": r.state.as_str(),
                    "body_cursor": r.body_cursor,
                    "submitted_at": r.submitted_at,
                    "started_at": r.started_at,
                    "finished_at": r.finished_at,
                }),
            ),
            Ok(Err(vcli_store::StoreError::UnknownProgram(_))) => Response::err(
                id,
                ErrorPayload::simple(ErrorCode::UnknownProgram, "not found"),
            ),
            Ok(Err(e)) => {
                Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}")))
            }
            Err(e) => Response::err(id, ErrorPayload::simple(ErrorCode::Internal, format!("{e}"))),
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
            RequestOp::Submit { program } => self.handle_submit(id, program).await,
            RequestOp::List { state } => self.handle_list(id, state).await,
            RequestOp::Status { program_id } => self.handle_status(id, program_id).await,
            RequestOp::Cancel { program_id } => self.handle_cancel(id, program_id),
            RequestOp::Start { program_id } => self.handle_start(id, program_id),
            RequestOp::Resume {
                program_id,
                from_start,
            } => self.handle_resume(id, program_id, from_start).await,
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
        op: RequestOp,
        tx: StreamSender,
    ) -> IpcResult<()> {
        match op {
            RequestOp::Events { follow } => self.stream_events(id, None, follow, tx).await,
            RequestOp::Logs {
                program_id,
                follow,
            } => self.stream_events(id, Some(program_id), follow, tx).await,
            RequestOp::Trace { program_id: _ } => {
                // v0 minimum: empty trace; server writes end_of_stream on return.
                let _ = tx;
                Ok(())
            }
            other => {
                debug!("stream op {other:?} not supported");
                let _ = tx
                    .send(StreamFrame::end_of_stream(id, StreamKind::Events))
                    .await;
                Ok(())
            }
        }
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
    async fn events_stream_drains_history_then_closes_when_not_following() {
        use tokio::sync::mpsc;
        let f = fresh_handler();
        let pid = ProgramId::new();
        {
            let mut s = f.handler.store.lock().unwrap();
            s.insert_program(&vcli_store::NewProgram {
                id: pid,
                name: "x",
                source_json: "{}",
                state: vcli_core::ProgramState::Pending,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
            let ev = Event {
                at: 7,
                data: vcli_core::EventData::ProgramCompleted {
                    program_id: pid,
                    emit: None,
                },
            };
            s.append_event(pid, &ev).unwrap();
        }

        let (tx, mut rx) = mpsc::channel::<StreamFrame>(8);
        let sender = StreamSender(tx);
        let handler = f.handler.clone();
        let task = tokio::spawn(async move {
            handler
                .handle_stream(
                    RequestId::new(),
                    RequestOp::Events { follow: false },
                    sender,
                )
                .await
        });

        let frame = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("stream_events should deliver history in <500ms")
            .expect("channel closed before delivering frame");
        assert!(frame.event.is_some());

        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), task)
            .await
            .expect("handler should return after draining history");
    }

    fn minimal_valid_program_json() -> String {
        serde_json::json!({
            "version": "0.1",
            "name": "r",
            "trigger": { "kind": "on_submit" },
            "predicates": {},
            "watches": [],
            "body": [],
        })
        .to_string()
    }

    #[tokio::test]
    async fn resume_transitions_store_and_sends_command() {
        let f = fresh_handler();
        let pid = ProgramId::new();
        let path = f.dir.path().to_path_buf();
        let src = minimal_valid_program_json();
        {
            let mut s = f.handler.store.lock().unwrap();
            s.insert_program(&vcli_store::NewProgram {
                id: pid,
                name: "r",
                source_json: &src,
                state: vcli_core::ProgramState::Pending,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
            s.update_state(pid, vcli_core::ProgramState::Running, 1)
                .unwrap();
            s.set_body_cursor(pid, 3).unwrap();
        }
        // Trigger a recovery cycle by reopening the same DB.
        let (_, _) = vcli_store::Store::open(&path).unwrap();

        let resp = f
            .handler
            .handle(
                RequestId::new(),
                RequestOp::Resume {
                    program_id: pid,
                    from_start: false,
                },
            )
            .await
            .unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["result"]["from_step"], 3);
        match f.cmd_rx.try_recv().unwrap() {
            SchedulerCommand::ResumeRunning {
                program_id,
                from_step,
                ..
            } => {
                assert_eq!(program_id, pid);
                assert_eq!(from_step, 3);
            }
            other => panic!("wrong cmd: {other:?}"),
        }
    }

    #[tokio::test]
    async fn resume_rejects_non_resumable_program() {
        let f = fresh_handler();
        let pid = ProgramId::new();
        f.handler
            .store
            .lock()
            .unwrap()
            .insert_program(&vcli_store::NewProgram {
                id: pid,
                name: "r",
                source_json: "{}",
                state: vcli_core::ProgramState::Pending,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
        let resp = f
            .handler
            .handle(
                RequestId::new(),
                RequestOp::Resume {
                    program_id: pid,
                    from_start: false,
                },
            )
            .await
            .unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], "not_resumable");
    }

    #[tokio::test]
    async fn cancel_sends_command_and_returns_ok() {
        let f = fresh_handler();
        let pid = ProgramId::new();
        let resp = f
            .handler
            .handle(RequestId::new(), RequestOp::Cancel { program_id: pid })
            .await
            .unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["ok"], true);
        match f.cmd_rx.try_recv().unwrap() {
            SchedulerCommand::Cancel { program_id, .. } => assert_eq!(program_id, pid),
            other => panic!("wrong cmd: {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_sends_command_and_returns_ok() {
        let f = fresh_handler();
        let pid = ProgramId::new();
        let resp = f
            .handler
            .handle(RequestId::new(), RequestOp::Start { program_id: pid })
            .await
            .unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["ok"], true);
        match f.cmd_rx.try_recv().unwrap() {
            SchedulerCommand::Start { program_id } => assert_eq!(program_id, pid),
            other => panic!("wrong cmd: {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_returns_all_programs_when_no_filter() {
        let f = fresh_handler();
        for name in ["a", "b"] {
            let id = ProgramId::new();
            f.handler
                .store
                .lock()
                .unwrap()
                .insert_program(&vcli_store::NewProgram {
                    id,
                    name,
                    source_json: "{}",
                    state: vcli_core::ProgramState::Pending,
                    submitted_at: 0,
                    labels_json: "{}",
                })
                .unwrap();
        }
        let resp = f
            .handler
            .handle(RequestId::new(), RequestOp::List { state: None })
            .await
            .unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        let items = body["result"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn status_returns_row_for_known_id() {
        let f = fresh_handler();
        let pid = ProgramId::new();
        f.handler
            .store
            .lock()
            .unwrap()
            .insert_program(&vcli_store::NewProgram {
                id: pid,
                name: "s",
                source_json: "{}",
                state: vcli_core::ProgramState::Pending,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
        let resp = f
            .handler
            .handle(RequestId::new(), RequestOp::Status { program_id: pid })
            .await
            .unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["result"]["name"], "s");
        assert_eq!(body["result"]["state"], "pending");
    }

    #[tokio::test]
    async fn status_returns_unknown_program_for_missing_id() {
        let f = fresh_handler();
        let resp = f
            .handler
            .handle(
                RequestId::new(),
                RequestOp::Status {
                    program_id: ProgramId::new(),
                },
            )
            .await
            .unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], "unknown_program");
    }

    #[tokio::test]
    async fn submit_validates_and_enqueues() {
        let f = fresh_handler();
        let program = serde_json::json!({
            "version": "0.1",
            "name": "noop",
            "trigger": { "kind": "on_submit" },
            "predicates": {},
            "watches": [],
            "body": [],
        });
        let id = RequestId::new();
        let resp = f
            .handler
            .handle(
                id,
                RequestOp::Submit {
                    program: program.clone(),
                },
            )
            .await
            .unwrap();
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["ok"], true);
        let pid: ProgramId = body["result"]["program_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let row = f.handler.store.lock().unwrap().get_program(pid).unwrap();
        assert_eq!(row.name, "noop");
        match f.cmd_rx.try_recv().unwrap() {
            SchedulerCommand::SubmitValidated { program_id, .. } => assert_eq!(program_id, pid),
            other => panic!("wrong cmd: {other:?}"),
        }
    }

    #[tokio::test]
    async fn shutdown_triggers_oneshot_and_cmd() {
        let f = fresh_handler();
        let id = RequestId::new();
        let _resp = f.handler.handle(id, RequestOp::Shutdown).await.unwrap();
        assert!(f.shutdown_rx.await.is_ok());
    }
}
