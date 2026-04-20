//! On shutdown, the socket file and pidfile should both be unlinked.

use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;

use vcli_daemon::{run_foreground, Config, RuntimeBackends};
use vcli_ipc::socket_path::SocketPathOrigin;
use vcli_ipc::{IpcClient, RequestOp, ResponseBody};

fn mocks() -> Result<RuntimeBackends, vcli_daemon::DaemonError> {
    Ok(RuntimeBackends {
        capture: Box::new(vcli_capture::MockCapture::empty()),
        input: Arc::new(vcli_input::MockInputSink::new()),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_runtime::SystemRuntimeClock::new()),
    })
}

#[tokio::test]
async fn shutdown_unlinks_socket_file() {
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
    let r = client.request(RequestOp::Shutdown).await.unwrap();
    assert!(matches!(r.body, ResponseBody::Ok { .. }));

    tokio::time::timeout(Duration::from_secs(2), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert!(!sock.exists(), "socket should be unlinked after shutdown");
    let pidfile = dir.path().join("data").join("daemon.pid");
    assert!(!pidfile.exists(), "pidfile should be removed after shutdown");
}
