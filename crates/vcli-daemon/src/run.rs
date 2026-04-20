//! Daemon assembly. `run_foreground` is the function the binary calls. All
//! meaningful side effects (opening the store, binding the socket, spawning
//! the scheduler thread, installing signals) happen inside here, in exactly
//! the order spec §Restart semantics Phase A requires.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use tokio::sync::oneshot;
use tracing::{error, info};

use vcli_capture::Capture;
use vcli_input::InputSink;
use vcli_ipc::IpcServer;
use vcli_perception::Perception;
use vcli_store::Store;

use crate::bridge::{new_channels, SchedulerCommand};
use crate::config::{ensure_dirs, Config};
use crate::error::{DaemonError, DaemonResult};
use crate::handler::DaemonHandler;
use crate::persist::spawn_event_pump;
use crate::pidfile::PidFile;
use crate::shutdown::{emit_daemon_stopped, install_signal_handlers};
use crate::startup::{emit_daemon_started, emit_recovery_events, reload_waiting_programs};

/// Bundle of backend implementations the daemon will hand to the scheduler.
pub struct RuntimeBackends {
    /// Capture backend.
    pub capture: Box<dyn Capture>,
    /// Input sink.
    pub input: Arc<dyn InputSink>,
    /// Perception façade.
    pub perception: Perception,
    /// Runtime clock (usually `vcli_runtime::SystemRuntimeClock`). The runtime
    /// crate exposes its own `RuntimeClock` trait because Rust 1.75 lacks
    /// trait-object upcasting, so we cannot pass a `vcli_core::Clock` here.
    pub clock: Arc<dyn vcli_runtime::RuntimeClock>,
}

/// Function that produces `RuntimeBackends` at startup. Lets tests inject
/// mocks without entering `run_foreground` through platform backends.
pub type RuntimeFactory = Box<dyn FnOnce() -> DaemonResult<RuntimeBackends> + Send>;

/// Foreground entrypoint. Blocks until a signal / IPC shutdown drains the
/// process. Use this from `fn main` after argv parsing.
///
/// # Errors
/// Startup failures (path resolution, already-running, store open, socket bind).
pub async fn run_foreground(cfg: Config, factory: RuntimeFactory) -> DaemonResult<()> {
    ensure_dirs(&cfg)?;
    let _log_guard = crate::logging::init(&cfg.log_dir)?;
    info!(
        data_root = %cfg.data_root.display(),
        socket = %cfg.socket.path.display(),
        "starting vcli-daemon"
    );

    let pid = PidFile::acquire(cfg.pidfile_path())?;
    info!(pid = pid.pid(), pidfile = %pid.path().display(), "pidfile acquired");

    let (store, recovered) = Store::open(&cfg.data_root)?;
    let store = Arc::new(Mutex::new(store));

    let (bridge, cmd_rx, event_rx, sched_event_tx) = new_channels();

    let pump = spawn_event_pump(store.clone(), event_rx, bridge.event_tx.clone());

    emit_recovery_events(&recovered, &sched_event_tx);
    emit_daemon_started(&sched_event_tx);

    let _ = reload_waiting_programs(&store, &bridge.cmd_tx);

    let RuntimeBackends {
        capture,
        input,
        perception,
        clock,
    } = factory()?;
    let sched_event_tx_for_thread = sched_event_tx.clone();
    let scheduler_join = thread::Builder::new()
        .name("vcli-scheduler".into())
        .spawn(move || {
            let scheduler = vcli_runtime::Scheduler::new(
                vcli_runtime::SchedulerConfig::default(),
                capture,
                input,
                perception,
                clock,
                cmd_rx,
                sched_event_tx_for_thread,
            );
            scheduler.run_until_shutdown();
        })
        .map_err(DaemonError::Io)?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_slot = Arc::new(Mutex::new(Some(shutdown_tx)));
    let handler = DaemonHandler {
        store: store.clone(),
        bridge: bridge.clone(),
        started_at: Instant::now(),
        shutdown_tx: shutdown_slot.clone(),
    };

    install_signal_handlers(shutdown_slot.clone())
        .await
        .map_err(DaemonError::Io)?;

    let server = IpcServer::bind(&cfg.socket.path, Arc::new(handler))?;
    info!(socket = %cfg.socket.path.display(), "listening");

    let serve_result = server.serve(shutdown_rx).await;
    if let Err(e) = serve_result {
        error!(error = %e, "ipc server terminated with error");
    }

    let _ = bridge.cmd_tx.send(SchedulerCommand::Shutdown);
    emit_daemon_stopped(&sched_event_tx);
    drop(sched_event_tx);
    if let Err(e) = scheduler_join.join() {
        error!("scheduler thread panicked: {e:?}");
    }
    let _ = pump.await;
    pid.release()?;
    info!("vcli-daemon exited cleanly");
    Ok(())
}
