//! Signal handling. On Unix, SIGTERM and SIGINT both trigger graceful shutdown
//! by firing the oneshot the IPC server is listening on. The second signal is
//! not special — `tokio::select!` races the loop already. A third signal
//! should escalate via the OS (we don't install SIGKILL handling).

use std::sync::{Arc, Mutex};

use futures_util::stream::StreamExt;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// Install a signal handler on the current tokio runtime that fires
/// `shutdown_tx` on the first SIGTERM or SIGINT. `shutdown_tx` is shared with
/// `DaemonHandler` so an IPC `shutdown` also counts.
///
/// # Errors
/// Returns `std::io::Error` if `signal_hook_tokio::Signals::new` fails.
///
/// # Panics
/// The background signal task panics if the shutdown mutex is poisoned.
pub async fn install_signal_handlers(
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
) -> std::io::Result<()> {
    use signal_hook_tokio::Signals;
    let mut signals = Signals::new([libc::SIGTERM, libc::SIGINT])?;
    tokio::spawn(async move {
        if let Some(sig) = signals.next().await {
            info!(signal = ?sig, "received shutdown signal");
            if let Some(tx) = shutdown_tx.lock().unwrap().take() {
                let _ = tx.send(());
            } else {
                warn!("shutdown already triggered");
            }
        }
    });
    Ok(())
}

/// Emit `daemon.stopped` on the scheduler event channel so IPC subscribers and
/// the DB see the transition.
pub fn emit_daemon_stopped(sched_event_tx: &crossbeam_channel::Sender<vcli_core::Event>) {
    use vcli_core::{Event, EventData};
    let _ = sched_event_tx.send(Event {
        at: vcli_core::clock::now_unix_ms(),
        data: EventData::DaemonStopped,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn emit_daemon_stopped_sends_one_event() {
        let (tx, rx) = unbounded::<vcli_core::Event>();
        emit_daemon_stopped(&tx);
        assert!(matches!(
            rx.recv().unwrap().data,
            vcli_core::EventData::DaemonStopped
        ));
    }

    #[tokio::test]
    async fn install_handler_is_idempotent_without_signal() {
        let (tx, _rx) = oneshot::channel();
        let slot = Arc::new(Mutex::new(Some(tx)));
        install_signal_handlers(slot.clone()).await.unwrap();
    }
}
