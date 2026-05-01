//! End-to-end: start the daemon with mock backends, submit a minimal program
//! over IPC, assert the daemon returns a program_id, then shut down.

use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use tokio::time::timeout;

use vcli_daemon::{run_foreground, Config, RuntimeBackends};
use vcli_ipc::socket_path::SocketPathOrigin;
use vcli_ipc::{IpcClient, RequestOp, ResponseBody};

fn mocks() -> Result<RuntimeBackends, vcli_daemon::DaemonError> {
    Ok(RuntimeBackends {
        capture: Box::new(vcli_capture::MockCapture::empty()),
        input: Arc::new(vcli_input::MockInputSink::new()),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_runtime::SystemRuntimeClock::new()),
        _shutdown_guard: None,
    })
}

#[tokio::test]
async fn submit_creates_program_row_and_returns_id() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("vcli.sock");
    let cfg = Config::with_roots(
        dir.path().join("data"),
        dir.path().join("logs"),
        vcli_ipc::SocketPath {
            path: sock.clone(),
            origin: SocketPathOrigin::Override,
        },
    );

    let factory: vcli_daemon::RuntimeFactory = Box::new(mocks);
    let run = tokio::spawn(run_foreground(cfg, factory));

    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(sock.exists());

    let mut client = IpcClient::connect(&sock).await.unwrap();
    let program = serde_json::json!({
        "version": "0.1",
        "name": "noop",
        "trigger": { "kind": "on_submit" },
        "predicates": {},
        "watches": [],
        "body": [],
    });
    let resp = client
        .request(RequestOp::Submit {
            program,
            base_dir: None,
        })
        .await
        .unwrap();
    match resp.body {
        ResponseBody::Ok { result, .. } => {
            let pid = result["program_id"].as_str().unwrap();
            assert!(!pid.is_empty());
        }
        ResponseBody::Err { error, .. } => panic!("submit failed: {error:?}"),
    }

    let shut = client.request(RequestOp::Shutdown).await.unwrap();
    assert!(matches!(shut.body, ResponseBody::Ok { .. }));

    timeout(Duration::from_secs(2), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
