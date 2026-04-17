//! Clock abstraction. Prod = `SystemClock`; tests = `TestClock` for determinism.
//!
//! Per Decision 1.6 every time-reading site in vcli takes `&dyn Clock` (or a
//! generic `C: Clock`). Scheduler throttles, elapsed-since-true, and timeouts
//! all resolve via this trait so `TestClock::advance_by(…)` drives them
//! deterministically in tests.
//!
//! Wall-clock / timezone reads belong to a separate `WallClock` trait — NOT
//! in v0 (see TODOS.md "`WallClock` trait + `on_schedule` trigger").

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Unix milliseconds since epoch. Used for event timestamps and `SQLite` rows.
pub type UnixMs = i64;

/// Monotonic time source. Never goes backwards. The returned `Duration` is
/// from an arbitrary fixed epoch — only differences are meaningful.
pub trait Clock: Send + Sync {
    /// Monotonic time reading.
    fn now(&self) -> Duration;

    /// Wall-clock reading for event timestamps.
    /// Implementations should use the same logical "now" for both calls when
    /// possible, but drift between the two is acceptable.
    fn unix_ms(&self) -> UnixMs;
}

/// Production clock. Backed by `std::time::Instant` + `SystemTime::now`.
#[derive(Debug)]
pub struct SystemClock {
    epoch: Instant,
}

impl SystemClock {
    /// Create a new clock whose `now()` reads relative to process start.
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    fn unix_ms(&self) -> UnixMs {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        i64::try_from(now.as_millis()).unwrap_or(i64::MAX)
    }
}

/// Deterministic test clock. Both `now()` and `unix_ms()` advance by the same
/// amount when `advance_by` is called.
#[derive(Debug)]
pub struct TestClock {
    inner: Mutex<TestClockInner>,
}

#[derive(Debug)]
struct TestClockInner {
    now: Duration,
    unix_ms: UnixMs,
}

impl TestClock {
    /// Create a clock at `Duration::ZERO` monotonic and at the given unix-ms baseline.
    #[must_use]
    pub fn at_unix_ms(baseline: UnixMs) -> Self {
        Self {
            inner: Mutex::new(TestClockInner {
                now: Duration::ZERO,
                unix_ms: baseline,
            }),
        }
    }

    /// Advance both clocks by `d`.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn advance_by(&self, d: Duration) {
        let mut g = self.inner.lock().unwrap();
        g.now = g.now.saturating_add(d);
        let add = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
        g.unix_ms = g.unix_ms.saturating_add(add);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.inner.lock().unwrap().now
    }

    fn unix_ms(&self) -> UnixMs {
        self.inner.lock().unwrap().unix_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_monotonic_advance() {
        let c = SystemClock::new();
        let a = c.now();
        std::thread::sleep(Duration::from_millis(5));
        let b = c.now();
        assert!(b >= a, "clock went backwards: a={a:?}, b={b:?}");
    }

    #[test]
    fn test_clock_starts_at_zero_monotonic() {
        let c = TestClock::at_unix_ms(1_700_000_000_000);
        assert_eq!(c.now(), Duration::ZERO);
        assert_eq!(c.unix_ms(), 1_700_000_000_000);
    }

    #[test]
    fn test_clock_advance_moves_both_readings() {
        let c = TestClock::at_unix_ms(0);
        c.advance_by(Duration::from_secs(1));
        assert_eq!(c.now(), Duration::from_secs(1));
        assert_eq!(c.unix_ms(), 1_000);
        c.advance_by(Duration::from_millis(250));
        assert_eq!(c.now(), Duration::from_millis(1_250));
        assert_eq!(c.unix_ms(), 1_250);
    }

    #[test]
    fn test_clock_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TestClock>();
        assert_send_sync::<SystemClock>();
    }
}
