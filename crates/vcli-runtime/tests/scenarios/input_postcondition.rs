//! Scenario: Click → WaitFor(postcondition) succeeds when the postcondition
//! flips truthy.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vcli_core::action::Button;
use vcli_core::geom::Point;
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::step::{OnTimeout, Step, Target};
use vcli_core::{program::DslVersion, trigger::Trigger, Program};
use vcli_perception::Perception;
use vcli_runtime::clock::ManualClock;

#[test]
fn click_then_wait_for_new_state_succeeds() {
    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let green = ScriptedCapture::solid(1, 1, [0, 255, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red, green]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert(
        "green".into(),
        PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([0, 255, 0]),
            tolerance: 0,
        },
    );
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "postcond".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![],
        body: vec![
            Step::Click {
                at: Target::Absolute(Point { x: 0, y: 0 }),
                button: Button::Left,
            },
            Step::WaitFor {
                predicate: "green".into(),
                timeout_ms: 300,
                on_timeout: OnTimeout::Fail,
            },
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
        input.clone(),
        Perception::new(),
        clock,
        cmd_rx,
        ev_tx,
    );
    cmd_tx
        .send(SchedulerCommand::SubmitValidated {
            program_id: id,
            program,
            assets: BTreeMap::new(),
        })
        .unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(400));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    assert_eq!(
        events
            .iter()
            .filter(|e| event_type(e) == "program.completed")
            .count(),
        1
    );
    assert_eq!(
        input
            .calls()
            .iter()
            .filter(|c| matches!(c, common::mock_input::Call::Click(_, _)))
            .count(),
        1
    );
}
