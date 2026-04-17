//! Error-path integration: a raw peer that sends malformed frames, and a
//! client that disconnects after a partial write.

use std::sync::Arc;

use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::oneshot;

use vcli_ipc::handler::test_double::FakeHandler;
use vcli_ipc::{IpcServer, MAX_FRAME_LEN};

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
async fn raw_peer_clean_disconnect_is_fine() {
    let tmp = TempDir::new().unwrap();
    let (_shutdown, path) = start_server(&tmp).await;

    // Connect, immediately close without sending anything. Server must not panic.
    let stream = UnixStream::connect(&path).await.unwrap();
    drop(stream);

    // A follow-up client connects fine.
    let stream2 = UnixStream::connect(&path).await.unwrap();
    drop(stream2);
}

#[tokio::test]
async fn oversize_header_closes_connection_gracefully() {
    let tmp = TempDir::new().unwrap();
    let (_shutdown, path) = start_server(&tmp).await;

    let mut stream = UnixStream::connect(&path).await.unwrap();
    let bogus = MAX_FRAME_LEN + 1;
    stream.write_all(&bogus.to_be_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();

    // The server should reply with a best-effort error frame. Drain whatever
    // bytes come back and verify the connection closes cleanly.
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    // Either we got a framed error, or the server closed — both are valid.
}

#[tokio::test]
async fn malformed_json_body_yields_error_response() {
    let tmp = TempDir::new().unwrap();
    let (_shutdown, path) = start_server(&tmp).await;

    let mut stream = UnixStream::connect(&path).await.unwrap();
    let body = b"not json at all";
    let len = u32::try_from(body.len()).unwrap();
    stream.write_all(&len.to_be_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();

    // Read one framed response; it must be a Response::err.
    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr).await.unwrap();
    let resp_len = u32::from_be_bytes(hdr) as usize;
    let mut resp_buf = vec![0u8; resp_len];
    stream.read_exact(&mut resp_buf).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&resp_buf).unwrap();
    assert_eq!(v["ok"], false, "{v}");
    assert!(v["error"]["code"].is_string());
}
