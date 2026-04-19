//! Scheduler-facing clock. Combines a wall-clock read with a blocking sleep so
//! tests can substitute a manual clock that advances time without actually
//! sleeping.
//!
//! Not a supertrait of `vcli_core::Clock` — Rust 1.75 lacks trait-object
//! upcasting. Implementors forward `unix_ms` to an inner `vcli_core::Clock` or
//! compute it directly.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use vcli_core::{Clock, SystemClock, UnixMs};

/// Scheduler-facing clock. `Send + Sync` because the scheduler holds it behind
/// `Arc<dyn RuntimeClock>`.
pub trait RuntimeClock: Send + Sync {
    /// Wall-clock reading for event timestamps.
    fn unix_ms(&self) -> UnixMs;
    /// Block for `ms` milliseconds. `ManualClock` short-circuits under tests.
    fn sleep_ms(&self, ms: u32);
}

/// Production clock: wraps `vcli_core::SystemClock` + `std::thread::sleep`.
#[derive(Debug, Default)]
pub struct SystemRuntimeClock {
    inner: SystemClock,
}

impl SystemRuntimeClock {
    /// Fresh instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: SystemClock::new(),
        }
    }
}

impl RuntimeClock for SystemRuntimeClock {
    fn unix_ms(&self) -> UnixMs {
        self.inner.unix_ms()
    }
    fn sleep_ms(&self, ms: u32) {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }
}

/// Deterministic clock for tests. `unix_ms()` returns whatever the test set
/// via `set_unix_ms`; `sleep_ms` advances the clock by the same amount instead
/// of blocking.
#[derive(Debug, Clone)]
pub struct ManualClock {
    inner: Arc<(Mutex<UnixMs>, Condvar)>,
}

impl ManualClock {
    /// Create at `start_ms`.
    #[must_use]
    pub fn new(start_ms: UnixMs) -> Self {
        Self {
            inner: Arc::new((Mutex::new(start_ms), Condvar::new())),
        }
    }

    /// Jump the clock to `ms`.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn set_unix_ms(&self, ms: UnixMs) {
        let (lock, cv) = &*self.inner;
        *lock.lock().unwrap() = ms;
        cv.notify_all();
    }

    /// Advance the clock by `delta_ms`.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn advance_ms(&self, delta_ms: u32) {
        let (lock, cv) = &*self.inner;
        let mut g = lock.lock().unwrap();
        *g = g.saturating_add(UnixMs::from(delta_ms));
        cv.notify_all();
    }
}

impl RuntimeClock for ManualClock {
    fn unix_ms(&self) -> UnixMs {
        *self.inner.0.lock().unwrap()
    }
    fn sleep_ms(&self, ms: u32) {
        self.advance_ms(ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_advance_adds_time() {
        let c = ManualClock::new(1_000);
        c.advance_ms(250);
        assert_eq!(c.unix_ms(), 1_250);
    }

    #[test]
    fn manual_clock_sleep_advances_instead_of_blocks() {
        let c = ManualClock::new(0);
        let t0 = std::time::Instant::now();
        c.sleep_ms(10_000);
        assert!(
            t0.elapsed() < Duration::from_millis(100),
            "sleep must not block"
        );
        assert_eq!(c.unix_ms(), 10_000);
    }

    #[test]
    fn system_runtime_clock_is_monotonic_within_a_tick() {
        let c = SystemRuntimeClock::new();
        let t1 = c.unix_ms();
        let t2 = c.unix_ms();
        assert!(t2 >= t1);
    }
}
