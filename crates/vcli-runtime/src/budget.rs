//! Per-tick budget tracking. Spec §348 (capture overrun) + Decision 4.1 (daemon.pressure).

use std::time::Instant;

/// Tick budget gate.
pub struct BudgetGate {
    /// Configured budget (ms).
    pub budget_ms: u32,
    /// Deadline for the current tick.
    deadline: Option<Instant>,
    /// Count of over-budget ticks since last reset (for `daemon.pressure` emission).
    overrun_streak: u32,
}

impl BudgetGate {
    /// Create with `budget_ms`.
    #[must_use]
    pub fn new(budget_ms: u32) -> Self {
        Self {
            budget_ms,
            deadline: None,
            overrun_streak: 0,
        }
    }

    /// Start a new tick. Call at the top of `tick()`.
    pub fn start_tick(&mut self) {
        self.deadline =
            Some(Instant::now() + std::time::Duration::from_millis(u64::from(self.budget_ms)));
    }

    /// Whether the current tick has exceeded its budget.
    #[must_use]
    pub fn is_over(&self) -> bool {
        match self.deadline {
            Some(d) => Instant::now() > d,
            None => false,
        }
    }

    /// Increment or reset the overrun streak. Returns `true` if this tick
    /// crossed the pressure threshold (5 consecutive overruns per Decision 4.1).
    pub fn note_outcome(&mut self, overrun: bool) -> bool {
        if overrun {
            self.overrun_streak += 1;
        } else {
            self.overrun_streak = 0;
        }
        self.overrun_streak >= 5
    }

    /// Reset streak without noting a new tick (used by tests).
    pub fn reset_streak(&mut self) {
        self.overrun_streak = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_threshold_at_five() {
        let mut b = BudgetGate::new(90);
        for _ in 0..4 {
            assert!(!b.note_outcome(true));
        }
        assert!(b.note_outcome(true));
    }

    #[test]
    fn successful_tick_resets_streak() {
        let mut b = BudgetGate::new(90);
        for _ in 0..4 {
            b.note_outcome(true);
        }
        b.note_outcome(false);
        assert!(!b.note_outcome(true));
    }

    #[test]
    fn is_over_false_before_start_tick() {
        let b = BudgetGate::new(90);
        assert!(!b.is_over());
    }
}
