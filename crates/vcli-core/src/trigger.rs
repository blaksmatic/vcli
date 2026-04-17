//! Program start triggers. `on_schedule` is deliberately absent in v0 — it
//! requires a `WallClock` trait (see TODOS.md) and lands post-v0.

use serde::{Deserialize, Serialize};

/// How a program transitions from `waiting` into `running`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Fire immediately once the daemon is ready and the program has been loaded.
    OnSubmit,
    /// Fire when the named predicate becomes truthy.
    OnPredicate {
        /// Predicate name in the same program.
        name: String,
    },
    /// Stay in `waiting` until `vcli start <id>`.
    Manual,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_submit_roundtrip() {
        let t = Trigger::OnSubmit;
        let j = serde_json::to_string(&t).unwrap();
        assert_eq!(j, r#"{"kind":"on_submit"}"#);
        let back: Trigger = serde_json::from_str(&j).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn on_predicate_roundtrip() {
        let t = Trigger::OnPredicate {
            name: "ready".into(),
        };
        let j = serde_json::to_string(&t).unwrap();
        let back: Trigger = serde_json::from_str(&j).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn manual_roundtrip() {
        let t = Trigger::Manual;
        let back: Trigger = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn on_schedule_is_rejected() {
        // Decision D removed on_schedule from v0. Defensive: serde shouldn't accept it.
        let j = r#"{"kind":"on_schedule","cron":"0 21 * * *"}"#;
        let r: Result<Trigger, _> = serde_json::from_str(j);
        assert!(r.is_err(), "on_schedule must not parse in v0");
    }
}
