//! Event emitter: wraps a `crossbeam_channel::Sender<Event>` and stamps every
//! emission with `clock.unix_ms()`.

use std::sync::Arc;

use crossbeam_channel::Sender;
use vcli_core::{Event, EventData};

use crate::clock::RuntimeClock;

/// Stamp + send. The daemon owns the receiver side.
#[derive(Clone)]
pub struct EventEmitter {
    tx: Sender<Event>,
    clock: Arc<dyn RuntimeClock>,
}

impl EventEmitter {
    /// Constructor.
    #[must_use]
    pub fn new(tx: Sender<Event>, clock: Arc<dyn RuntimeClock>) -> Self {
        Self { tx, clock }
    }

    /// Emit `data` stamped with `clock.unix_ms()`. Returns `false` only if the
    /// receiver has been dropped (daemon shut down before us). The scheduler
    /// ignores the return value and keeps running; the daemon drains
    /// remaining events after joining the scheduler thread.
    pub fn emit(&self, data: EventData) -> bool {
        let ev = Event {
            at: self.clock.unix_ms(),
            data,
        };
        self.tx.send(ev).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crossbeam_channel::unbounded;
    use vcli_core::ProgramId;

    fn sample_id() -> ProgramId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn emit_stamps_with_clock_now() {
        let (tx, rx) = unbounded::<Event>();
        let clock: Arc<dyn RuntimeClock> = Arc::new(ManualClock::new(12_345));
        let em = EventEmitter::new(tx, clock);
        assert!(em.emit(EventData::ProgramSubmitted {
            program_id: sample_id(),
            name: "x".into()
        }));
        let ev = rx.recv().unwrap();
        assert_eq!(ev.at, 12_345);
    }

    #[test]
    fn emit_returns_false_when_receiver_dropped() {
        let (tx, rx) = unbounded::<Event>();
        let clock: Arc<dyn RuntimeClock> = Arc::new(ManualClock::new(0));
        let em = EventEmitter::new(tx, clock);
        drop(rx);
        assert!(!em.emit(EventData::DaemonStopped));
    }
}
