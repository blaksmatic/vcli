//! `SchedulerCommand` — messages sent from the daemon's tokio reactor into the
//! synchronous scheduler thread. Shape is pinned by spec §Threading model and
//! by the sibling vcli-daemon plan.

use vcli_core::{Program, ProgramId};

/// Commands consumed on every tick before evaluation.
#[derive(Debug, Clone)]
pub enum SchedulerCommand {
    /// Program was already validated by `vcli-dsl` and persisted by the
    /// daemon. The scheduler inserts it at state `Pending` and fires the
    /// trigger next tick.
    SubmitValidated {
        /// Daemon-assigned id.
        program_id: ProgramId,
        /// Parsed program.
        program: Program,
    },
    /// Terminate a program with `Cancelled`.
    Cancel {
        /// Target.
        program_id: ProgramId,
        /// Human reason (propagated into `program.state_changed.reason`).
        reason: String,
    },
    /// Force-start a `Waiting` program (bypasses its trigger — used by
    /// `vcli resume`).
    Start {
        /// Target.
        program_id: ProgramId,
    },
    /// Re-insert a program at `Running` with a prior `body_cursor`.
    ResumeRunning {
        /// Target.
        program_id: ProgramId,
        /// Step index to resume at (`0` = from start).
        from_step: u32,
        /// Full program (daemon reloaded it from `SQLite`).
        program: Program,
    },
    /// Drain and exit `run_until_shutdown`.
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vcli_core::{program::DslVersion, trigger::Trigger};

    fn sample_program() -> Program {
        Program {
            version: DslVersion(DslVersion::V0_1.to_string()),
            name: "x".into(),
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
    fn submit_variant_roundtrips_basic_shape() {
        let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
        let c = SchedulerCommand::SubmitValidated {
            program_id: id,
            program: sample_program(),
        };
        match c {
            SchedulerCommand::SubmitValidated { program_id, .. } => assert_eq!(program_id, id),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn cancel_keeps_reason_untouched() {
        let id: ProgramId = "00000000-0000-4000-8000-000000000000".parse().unwrap();
        let c = SchedulerCommand::Cancel {
            program_id: id,
            reason: "user".into(),
        };
        match c {
            SchedulerCommand::Cancel { reason, .. } => assert_eq!(reason, "user"),
            _ => panic!("wrong variant"),
        }
    }
}
