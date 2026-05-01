//! Scenario: `ResumeRunning` starts the body at `from_step` and emits a
//! `program.resumed` event.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vcli_core::step::Step;
use vcli_core::{program::DslVersion, trigger::Trigger, Program};
use vcli_perception::Perception;
use vcli_runtime::clock::ManualClock;

#[test]
fn resume_running_starts_body_at_cursor_and_emits_resumed() {
    let blank = ScriptedCapture::solid(1, 1, [0, 0, 0, 0]);
    let capture = Box::new(ScriptedCapture::new(vec![blank]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "resumable".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: BTreeMap::new(),
        watches: vec![],
        body: vec![
            Step::SleepMs { ms: 1 },
            Step::SleepMs { ms: 1 },
            Step::SleepMs { ms: 1 },
        ],
        on_complete: None,
        on_fail: None,
        timeout_ms: None,
        labels: BTreeMap::new(),
        priority: Default::default(),
    };

    let sched = Scheduler::new(
        SchedulerConfig::default(),
        capture,
        input,
        Perception::new(),
        clock,
        cmd_rx,
        ev_tx,
    );
    cmd_tx
        .send(SchedulerCommand::ResumeRunning {
            program_id: id,
            from_step: 2,
            program,
            assets: BTreeMap::new(),
        })
        .unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    let resumed = events.iter().find(|e| event_type(e) == "program.resumed");
    assert!(resumed.is_some(), "must emit program.resumed");
    assert_eq!(
        events
            .iter()
            .filter(|e| event_type(e) == "program.completed")
            .count(),
        1
    );
}
