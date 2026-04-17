//! End-to-end streaming path: FakeHandler emits events → server forwards →
//! client drains until end_of_stream.

use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::oneshot;

use vcli_ipc::handler::test_double::FakeHandler;
use vcli_ipc::{IpcClient, IpcServer, RequestOp, StreamKind};

async fn start_server(tmp: &TempDir) -> (oneshot::Sender<()>, std::path::PathBuf) {
    let path = tmp.path().join("vcli.sock");
    let handler = Arc::new(FakeHandler::default());
    let server = IpcServer::bind(&path, handler).unwrap();
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = server.serve(rx).await;
    });
    tokio::task::yield_now().await;
    (tx, path)
}

#[tokio::test]
async fn events_stream_receives_frames_then_end_of_stream() {
    let tmp = TempDir::new().unwrap();
    let (_shutdown, path) = start_server(&tmp).await;

    let client = IpcClient::connect(&path).await.unwrap();
    let mut stream = client
        .request_stream(RequestOp::Events { follow: false })
        .await
        .unwrap();

    let mut count = 0;
    while let Some(frame) = stream.next_frame().await.unwrap() {
        assert_eq!(frame.stream, StreamKind::Events);
        assert!(frame.event.is_some());
        count += 1;
    }
    // FakeHandler pushes two frames before returning.
    assert_eq!(count, 2);
}

#[tokio::test]
async fn client_drop_midstream_does_not_crash_server() {
    let tmp = TempDir::new().unwrap();
    let (_shutdown, path) = start_server(&tmp).await;

    // Connect, start stream, drop immediately.
    let client = IpcClient::connect(&path).await.unwrap();
    let stream = client.request_stream(RequestOp::Events { follow: false }).await.unwrap();
    drop(stream);

    // A second client should still be serviceable.
    let mut c2 = IpcClient::connect(&path).await.unwrap();
    let r = c2.request(RequestOp::Health).await.unwrap();
    assert_eq!(serde_json::to_value(&r.body).unwrap()["ok"], true);
}
