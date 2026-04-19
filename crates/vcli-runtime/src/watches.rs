//! Watch lifetime bookkeeping. Spec §416.

use vcli_core::watch::Lifetime;
use vcli_core::{UnixMs, Watch};

use crate::program::WatchRuntime;

/// Decision for this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchDecision {
    /// Do not fire and do not retire.
    Skip,
    /// Fire the watch (run its `do` steps).
    Fire,
    /// Retire without firing (e.g. `UntilPredicate` went truthy mid-tick).
    Retire,
}

/// Decide for a single watch given its current `state` and the *new* truthiness
/// result from this tick. Does NOT mutate; the scheduler applies mutation
/// (`state.last_truthy` / `state.last_fired_ms` / `state.retired`) only after
/// the watch's `do` steps actually dispatch (so a kill-switch or arbiter drop
/// does not consume the firing budget).
#[must_use]
pub fn decide(
    watch: &Watch,
    state: &WatchRuntime,
    truthy_now: bool,
    now_ms: UnixMs,
    running_since_ms: UnixMs,
) -> WatchDecision {
    if state.retired {
        return WatchDecision::Skip;
    }
    if let Lifetime::TimeoutMs { ms } = watch.lifetime {
        if now_ms.saturating_sub(running_since_ms) >= UnixMs::from(ms) {
            return WatchDecision::Retire;
        }
    }
    if !(truthy_now && !state.last_truthy) {
        return WatchDecision::Skip;
    }
    if let Some(last) = state.last_fired_ms {
        if now_ms.saturating_sub(last) < UnixMs::from(watch.throttle_ms) {
            return WatchDecision::Skip;
        }
    }
    WatchDecision::Fire
}

/// Apply post-firing retirement rules for `OneShot`.
pub fn after_fire(watch: &Watch, state: &mut WatchRuntime, now_ms: UnixMs) {
    state.last_fired_ms = Some(now_ms);
    if matches!(watch.lifetime, Lifetime::OneShot) {
        state.retired = true;
    }
}

/// Retire a watch when its `UntilPredicate` predicate is truthy this tick.
pub fn on_until_predicate_truthy(state: &mut WatchRuntime) {
    state.retired = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::watch::{Lifetime, WatchWhen};

    fn mk_watch(lifetime: Lifetime, throttle_ms: u32) -> Watch {
        Watch {
            when: WatchWhen::ByName("p".into()),
            steps: vec![],
            throttle_ms,
            lifetime,
        }
    }

    #[test]
    fn fires_on_false_to_true_edge() {
        let w = mk_watch(Lifetime::Persistent, 0);
        let st = WatchRuntime::default();
        assert_eq!(decide(&w, &st, true, 100, 0), WatchDecision::Fire);
    }

    #[test]
    fn skips_on_level_high() {
        let w = mk_watch(Lifetime::Persistent, 0);
        let st = WatchRuntime {
            last_truthy: true,
            ..Default::default()
        };
        assert_eq!(decide(&w, &st, true, 100, 0), WatchDecision::Skip);
    }

    #[test]
    fn throttle_suppresses_rapid_refires() {
        let w = mk_watch(Lifetime::Persistent, 500);
        let st = WatchRuntime {
            last_truthy: false,
            last_fired_ms: Some(100),
            retired: false,
        };
        assert_eq!(decide(&w, &st, true, 400, 0), WatchDecision::Skip);
        assert_eq!(decide(&w, &st, true, 700, 0), WatchDecision::Fire);
    }

    #[test]
    fn one_shot_retires_after_fire() {
        let w = mk_watch(Lifetime::OneShot, 0);
        let mut st = WatchRuntime::default();
        after_fire(&w, &mut st, 50);
        assert!(st.retired);
    }

    #[test]
    fn persistent_does_not_retire_after_fire() {
        let w = mk_watch(Lifetime::Persistent, 0);
        let mut st = WatchRuntime::default();
        after_fire(&w, &mut st, 50);
        assert!(!st.retired);
        assert_eq!(st.last_fired_ms, Some(50));
    }

    #[test]
    fn timeout_ms_retires_when_window_elapsed() {
        let w = mk_watch(Lifetime::TimeoutMs { ms: 1_000 }, 0);
        let st = WatchRuntime::default();
        assert_eq!(decide(&w, &st, true, 2_001, 1_000), WatchDecision::Retire);
    }

    #[test]
    fn retired_watch_is_inert() {
        let w = mk_watch(Lifetime::Persistent, 0);
        let st = WatchRuntime {
            retired: true,
            ..Default::default()
        };
        assert_eq!(decide(&w, &st, true, 100, 0), WatchDecision::Skip);
    }
}
