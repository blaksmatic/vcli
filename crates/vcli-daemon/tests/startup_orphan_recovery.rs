//! Startup integration: plant a `running` row in a store before the daemon
//! starts, then drive `run_foreground` until the socket is listening and
//! shutdown cleanly, asserting:
//!   * the row is now `failed(daemon_restart)`
//!   * the event log contains `program.state_changed` and `program.failed`.

use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;
use tokio::time::timeout;

use vcli_core::ProgramState;
use vcli_daemon::{run_foreground, Config, RuntimeBackends};
use vcli_ipc::socket_path::SocketPathOrigin;
use vcli_store::{NewProgram, Store};

fn noop_backends() -> Result<RuntimeBackends, vcli_daemon::DaemonError> {
    Ok(RuntimeBackends {
        capture: Box::new(vcli_capture::MockCapture::empty()),
        input: Arc::new(vcli_input::MockInputSink::new()),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_runtime::SystemRuntimeClock::new()),
    })
}

#[tokio::test]
async fn running_programs_transition_to_failed_on_startup() {
    let dir = tempdir().unwrap();
    let data_root = dir.path().join("data");
    let log_dir = dir.path().join("logs");
    let sock = dir.path().join("vcli.sock");

    let pid = vcli_core::ProgramId::new();
    {
        let (mut store, _) = Store::open(&data_root).unwrap();
        store
            .insert_program(&NewProgram {
                id: pid,
                name: "orphan",
                source_json: "{}",
                state: ProgramState::Pending,
                submitted_at: 0,
                labels_json: "{}",
            })
            .unwrap();
        store.update_state(pid, ProgramState::Running, 1).unwrap();
        store.set_body_cursor(pid, 2).unwrap();
    }

    let cfg = Config::with_roots(
        &data_root,
        &log_dir,
        vcli_ipc::SocketPath {
            path: sock.clone(),
            origin: SocketPathOrigin::Override,
        },
    );

    let factory: vcli_daemon::RuntimeFactory = Box::new(noop_backends);
    let run = tokio::spawn(run_foreground(cfg, factory));

    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(sock.exists(), "daemon never bound the socket");

    let mut client = vcli_ipc::IpcClient::connect(&sock).await.unwrap();
    let resp = client.request(vcli_ipc::RequestOp::Shutdown).await.unwrap();
    assert!(matches!(resp.body, vcli_ipc::ResponseBody::Ok { .. }));

    timeout(Duration::from_secs(2), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let (store, _) = Store::open(&data_root).unwrap();
    let row = store.get_program(pid).unwrap();
    assert_eq!(row.state, ProgramState::Failed);
    let events = store.stream_events(0, 100).unwrap();
    let types: Vec<&str> = events.iter().map(|e| e.type_tag.as_str()).collect();
    assert!(types.contains(&"program.state_changed"));
    assert!(types.contains(&"program.failed"));
}
