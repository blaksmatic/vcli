//! Process-global HIL "stop everything" flag (Codex Decision B).
//!
//! Cheap to clone (internal `Arc<AtomicBool>`). Every `InputSink` method must
//! check `is_engaged()` before calling into the OS and return `InputError::Halted`
//! if set. The flag is observable (`subscribe`) so higher-level runtimes can
//! cancel waits as soon as a human panics.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Thread-safe kill switch. Cloning shares the underlying flag.
#[derive(Debug, Clone, Default)]
pub struct KillSwitch {
    engaged: Arc<AtomicBool>,
    engaged_at_ns: Arc<AtomicU64>,
}

impl KillSwitch {
    /// Fresh switch, not engaged.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the switch. Idempotent. Stamps the engagement time (monotonic ns since
    /// an unspecified epoch) for observability.
    pub fn engage(&self) {
        let now_ns = duration_since_boot_ns();
        // CAS so we don't overwrite the original engagement timestamp on re-engage.
        let _ = self
            .engaged_at_ns
            .compare_exchange(0, now_ns, Ordering::SeqCst, Ordering::SeqCst);
        self.engaged.store(true, Ordering::SeqCst);
    }

    /// Clear the switch. Used only by tests and admin paths; production never clears.
    pub fn disengage(&self) {
        self.engaged.store(false, Ordering::SeqCst);
        self.engaged_at_ns.store(0, Ordering::SeqCst);
    }

    /// Non-blocking check.
    #[must_use]
    pub fn is_engaged(&self) -> bool {
        self.engaged.load(Ordering::SeqCst)
    }

    /// Observer handle for other crates (e.g. `vcli-runtime`) that want to wake
    /// their waits when the switch flips. Busy-wait implementation is acceptable
    /// because engagement is a rare, terminal event.
    #[must_use]
    pub fn subscribe(&self) -> KillSwitchObserver {
        KillSwitchObserver {
            switch: self.clone(),
        }
    }
}

/// Observer returned by [`KillSwitch::subscribe`].
#[derive(Debug, Clone)]
pub struct KillSwitchObserver {
    switch: KillSwitch,
}

impl KillSwitchObserver {
    /// Return immediately if engaged; otherwise poll every `poll` until
    /// `timeout` elapses. Returns whether the switch became engaged.
    #[must_use]
    pub fn wait_until_engaged(&self, timeout: Duration, poll: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.switch.is_engaged() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(poll.min(Duration::from_millis(50)));
        }
    }
}

fn duration_since_boot_ns() -> u64 {
    // Monotonic-ish "now" — does not need to be synced across machines.
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| {
            u64::try_from(d.as_nanos() & u128::from(u64::MAX)).unwrap_or(0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn fresh_switch_is_not_engaged() {
        let k = KillSwitch::new();
        assert!(!k.is_engaged());
    }

    #[test]
    fn engage_is_observable_from_clone() {
        let k = KillSwitch::new();
        let clone = k.clone();
        assert!(!clone.is_engaged());
        k.engage();
        assert!(clone.is_engaged());
    }

    #[test]
    fn engage_is_idempotent() {
        let k = KillSwitch::new();
        k.engage();
        let first = k.engaged_at_ns.load(Ordering::SeqCst);
        k.engage();
        let second = k.engaged_at_ns.load(Ordering::SeqCst);
        assert_eq!(first, second, "timestamp must not change on re-engage");
    }

    #[test]
    fn disengage_resets_flag() {
        let k = KillSwitch::new();
        k.engage();
        k.disengage();
        assert!(!k.is_engaged());
    }

    #[test]
    fn observer_returns_true_on_engage_before_timeout() {
        let k = KillSwitch::new();
        let obs = k.subscribe();
        let k2 = k.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            k2.engage();
        });
        assert!(obs.wait_until_engaged(Duration::from_millis(500), Duration::from_millis(5)));
        handle.join().unwrap();
    }

    #[test]
    fn observer_returns_false_on_timeout() {
        let k = KillSwitch::new();
        let obs = k.subscribe();
        assert!(!obs.wait_until_engaged(Duration::from_millis(20), Duration::from_millis(5)));
    }
}
