//! Tokio ↔ scheduler bridge.
//!
//! The scheduler thread is pure sync; the tokio reactor is async. We route
//! data between them via two `crossbeam_channel`s:
//!   * `cmd_tx` (tokio → sched): bounded, backpressure-free; handlers send
//!     [`SchedulerCommand`] values.
//!   * `event_rx` (sched → tokio): unbounded. A dedicated tokio task drains
//!     it and pushes each [`vcli_core::Event`] through the persistence sink
//!     and then into the [`tokio::sync::broadcast`] channel for connected
//!     IPC subscribers.
//!
//! `event_tx` (broadcast side) is cloned per IPC subscription; each streaming
//! handler subscribes, reads frames, and hangs up when the client drops. The
//! capacity of the broadcast (Decision 1.7) is 1024; overflowing clients
//! receive a `stream.dropped` notification inside the handler.

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use tokio::sync::broadcast;
use vcli_core::Event;

pub use vcli_runtime::SchedulerCommand;

/// Capacity of the broadcast channel fanned out to IPC subscribers.
pub const EVENT_BROADCAST_CAPACITY: usize = 1024;

/// Capacity of the command channel tokio → scheduler.
pub const CMD_CAPACITY: usize = 256;

/// All the channel endpoints together. Cheap to clone; references passed into
/// the handler and startup modules.
#[derive(Clone)]
pub struct CommandChannel {
    /// Tokio → scheduler commands.
    pub cmd_tx: Sender<SchedulerCommand>,
    /// Broadcast fanning out persisted events to IPC subscribers.
    pub event_tx: broadcast::Sender<Event>,
}

/// Construct a matched set of endpoints. The `cmd_rx` and `event_rx` returned
/// are handed to the scheduler thread + the event-pump task respectively.
#[must_use]
pub fn new_channels() -> (
    CommandChannel,
    Receiver<SchedulerCommand>,
    Receiver<Event>,
    Sender<Event>,
) {
    let (cmd_tx, cmd_rx) = bounded::<SchedulerCommand>(CMD_CAPACITY);
    let (sched_event_tx, event_rx) = unbounded::<Event>();
    let (bcast_tx, _) = broadcast::channel::<Event>(EVENT_BROADCAST_CAPACITY);
    (
        CommandChannel {
            cmd_tx,
            event_tx: bcast_tx,
        },
        cmd_rx,
        event_rx,
        sched_event_tx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::{EventData, ProgramId};

    #[test]
    fn command_channel_is_cloneable_and_reaches_scheduler() {
        let (chan, cmd_rx, _event_rx, _sched_event_tx) = new_channels();
        let chan2 = chan.clone();
        chan2
            .cmd_tx
            .send(SchedulerCommand::Cancel {
                program_id: ProgramId::new(),
                reason: "test".into(),
            })
            .unwrap();
        match cmd_rx.recv().unwrap() {
            SchedulerCommand::Cancel { .. } => {}
            other => panic!("unexpected cmd: {other:?}"),
        }
    }

    #[tokio::test]
    async fn broadcast_fanout_reaches_multiple_subscribers() {
        let (chan, _cmd_rx, _event_rx, _sched_event_tx) = new_channels();
        let mut rx1 = chan.event_tx.subscribe();
        let mut rx2 = chan.event_tx.subscribe();
        let ev = Event {
            at: 1,
            data: EventData::DaemonStopped,
        };
        chan.event_tx.send(ev.clone()).unwrap();
        assert_eq!(rx1.recv().await.unwrap(), ev);
        assert_eq!(rx2.recv().await.unwrap(), ev);
    }
}
