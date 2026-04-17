//! End-to-end: FakeHandler → IpcServer → IpcClient → Response.

use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::oneshot;

use vcli_ipc::handler::test_double::FakeHandler;
use vcli_ipc::{IpcClient, IpcServer, RequestOp};
use vcli_core::ProgramId;

async fn start_server(tmp: &TempDir) -> (oneshot::Sender<()>, Arc<FakeHandler>, std::path::PathBuf) {
    let path = tmp.path().join("vcli.sock");
    let handler = Arc::new(FakeHandler::default());
    let server = IpcServer::bind(&path, handler.clone()).unwrap();
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = server.serve(rx).await;
    });
    // Give the listener a moment to be ready.
    tokio::task::yield_now().await;
    (tx, handler, path)
}

#[tokio::test]
async fn submit_returns_ok_with_program_id() {
    let tmp = TempDir::new().unwrap();
    let (_shutdown, _handler, path) = start_server(&tmp).await;

    let mut client = IpcClient::connect(&path).await.unwrap();
    let resp = client
        .request(RequestOp::Submit {
            program: serde_json::json!({ "version": "0.1", "name": "x" }),
        })
        .await
        .unwrap();

    let body = serde_json::to_value(&resp.body).unwrap();
    assert_eq!(body["ok"], serde_json::Value::Bool(true), "{body}");
    assert!(body["result"]["program_id"].is_string());
}

#[tokio::test]
async fn list_health_and_cancel_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let (_shutdown, handler, path) = start_server(&tmp).await;

    let mut client = IpcClient::connect(&path).await.unwrap();

    let list = client.request(RequestOp::List { state: None }).await.unwrap();
    assert_eq!(serde_json::to_value(&list.body).unwrap()["ok"], true);

    let health = client.request(RequestOp::Health).await.unwrap();
    assert_eq!(serde_json::to_value(&health.body).unwrap()["ok"], true);

    let cancel = client
        .request(RequestOp::Cancel { program_id: ProgramId::new() })
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&cancel.body).unwrap()["ok"], true);

    // Handler saw all three ops in order.
    let seen = handler.received.lock().await;
    assert_eq!(seen.len(), 3);
}

#[tokio::test]
async fn multiple_requests_on_one_connection() {
    let tmp = TempDir::new().unwrap();
    let (_shutdown, _handler, path) = start_server(&tmp).await;

    let mut client = IpcClient::connect(&path).await.unwrap();
    for _ in 0..5 {
        let r = client.request(RequestOp::Health).await.unwrap();
        assert_eq!(serde_json::to_value(&r.body).unwrap()["ok"], true);
    }
}
