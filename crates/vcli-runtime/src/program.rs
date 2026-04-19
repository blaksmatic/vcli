//! `RunningProgram` — the scheduler's in-memory view of a submitted program.

use std::collections::HashMap;

use vcli_core::state::ProgramState;
use vcli_core::{Program, ProgramId, UnixMs};

/// Scheduler-owned per-program state.
pub struct RunningProgram {
    /// Assigned id.
    pub id: ProgramId,
    /// Parsed program.
    pub program: Program,
    /// Current lifecycle state.
    pub state: ProgramState,
    /// Wall-clock ms when the program entered `Running` (for `timeout_ms` + `Lifetime::TimeoutMs`).
    pub running_since_ms: Option<UnixMs>,
    /// Active body step index. `Some(n)` while advancing; `None` when body exhausted.
    pub body_cursor: Option<u32>,
    /// Per-watch bookkeeping, keyed by watch index. Populated on entry to
    /// `Running`; drained as watches retire.
    pub watch_state: HashMap<u32, WatchRuntime>,
    /// If set, the next successful transition emits `program.resumed{from_step}`.
    pub resumed_from: Option<u32>,
    /// Per-program body executor state (`SleepMs` / `WaitFor` accumulators).
    /// Persists across ticks so in-flight waits advance between frames.
    pub body_state: crate::body::BodyState,
}

/// Per-watch runtime state.
#[derive(Debug, Default, Clone)]
pub struct WatchRuntime {
    /// Last fire timestamp (for `throttle_ms`). None = has never fired.
    pub last_fired_ms: Option<UnixMs>,
    /// Last tick's truthiness result (for false→true edge detection).
    pub last_truthy: bool,
    /// Whether the watch has already been retired (`OneShot` or `UntilPredicate` tripped).
    pub retired: bool,
}

impl RunningProgram {
    /// Construct at `Pending` with default bookkeeping.
    #[must_use]
    pub fn pending(id: ProgramId, program: Program) -> Self {
        Self {
            id,
            program,
            state: ProgramState::Pending,
            running_since_ms: None,
            body_cursor: None,
            watch_state: HashMap::new(),
            resumed_from: None,
            body_state: crate::body::BodyState::default(),
        }
    }

    /// Whether `body_cursor` points past the last body step.
    #[must_use]
    pub fn body_complete(&self) -> bool {
        match self.body_cursor {
            Some(n) => usize::try_from(n).unwrap_or(usize::MAX) >= self.program.body.len(),
            None => self.program.body.is_empty(),
        }
    }

    /// Count of watches that are not yet retired.
    #[must_use]
    pub fn active_watch_count(&self) -> usize {
        self.watch_state.values().filter(|w| !w.retired).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vcli_core::{program::DslVersion, trigger::Trigger};

    fn sample_program() -> Program {
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: "t".into(),
            id: None,
            trigger: Trigger::OnSubmit,
            predicates: BTreeMap::new(),
            watches: vec![],
            body: vec![],
            on_complete: None,
            on_fail: None,
            timeout_ms: None,
            labels: BTreeMap::new(),
            priority: vcli_core::Priority::default(),
        }
    }

    #[test]
    fn pending_is_initial_state() {
        let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
        let rp = RunningProgram::pending(id, sample_program());
        assert_eq!(rp.state, ProgramState::Pending);
        assert!(rp.body_complete(), "empty body is trivially complete");
        assert_eq!(rp.active_watch_count(), 0);
    }

    #[test]
    fn body_complete_detects_exhausted_cursor() {
        let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
        let mut p = sample_program();
        p.body = vec![vcli_core::Step::SleepMs { ms: 10 }];
        let mut rp = RunningProgram::pending(id, p);
        rp.body_cursor = Some(0);
        assert!(!rp.body_complete());
        rp.body_cursor = Some(1);
        assert!(rp.body_complete());
    }
}
