//! Scenario: Two programs firing on the same tick arbitrate to one winner.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vcli_core::action::Button;
use vcli_core::geom::Point;
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::step::{Step, Target};
use vcli_core::watch::{Lifetime, Watch, WatchWhen};
use vcli_core::{program::DslVersion, trigger::Trigger, Program};
use vcli_perception::Perception;
use vcli_runtime::clock::ManualClock;

fn build_program(name: &str) -> Program {
    let mut preds = BTreeMap::new();
    preds.insert(
        "red".into(),
        PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([255, 0, 0]),
            tolerance: 0,
        },
    );
    Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: name.into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![Watch {
            when: WatchWhen::ByName("red".into()),
            steps: vec![Step::Click {
                at: Target::Absolute(Point { x: 0, y: 0 }),
                button: Button::Left,
            }],
            throttle_ms: 0,
            lifetime: Lifetime::OneShot,
        }],
        body: vec![],
        on_complete: None,
        on_fail: None,
        timeout_ms: None,
        labels: BTreeMap::new(),
        priority: Default::default(),
    }
}

#[test]
fn two_programs_same_frame_arbitrate_to_one_winner() {
    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red]));
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(0));

    let id1: ProgramId = "11111111-1111-4567-8910-111213141516".parse().unwrap();
    let id2: ProgramId = "22222222-2222-4567-8910-111213141516".parse().unwrap();

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
            program_id: id1,
            program: build_program("p1"),
            assets: BTreeMap::new(),
        })
        .unwrap();
    cmd_tx
        .send(SchedulerCommand::SubmitValidated {
            program_id: id2,
            program: build_program("p2"),
            assets: BTreeMap::new(),
        })
        .unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    let dispatched = events
        .iter()
        .filter(|e| event_type(e) == "action.dispatched")
        .count();
    let deferred = events
        .iter()
        .filter(|e| event_type(e) == "action.deferred")
        .count();
    assert_eq!(dispatched, 1, "exactly one winner");
    assert_eq!(deferred, 1, "exactly one loser");
}
