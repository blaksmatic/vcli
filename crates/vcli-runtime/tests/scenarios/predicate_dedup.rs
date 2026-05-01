//! Scenario: Two programs sharing a predicate both fire when the predicate
//! goes truthy — the arbiter does not suppress empty-step watches.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vcli_core::geom::Point;
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::program::Priority;
use vcli_core::watch::{Lifetime, Watch, WatchWhen};
use vcli_core::{program::DslVersion, trigger::Trigger, Program};
use vcli_perception::Perception;
use vcli_runtime::clock::ManualClock;

fn build_program(name: &str, pri: i32) -> Program {
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
            steps: vec![],
            throttle_ms: 0,
            lifetime: Lifetime::OneShot,
        }],
        body: vec![],
        on_complete: None,
        on_fail: None,
        timeout_ms: None,
        labels: BTreeMap::new(),
        priority: Priority(pri),
    }
}

#[test]
fn two_programs_same_predicate_both_fire() {
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
        input,
        Perception::new(),
        clock,
        cmd_rx,
        ev_tx,
    );
    cmd_tx
        .send(SchedulerCommand::SubmitValidated {
            program_id: id1,
            program: build_program("p1", 1),
            assets: BTreeMap::new(),
        })
        .unwrap();
    cmd_tx
        .send(SchedulerCommand::SubmitValidated {
            program_id: id2,
            program: build_program("p2", 2),
            assets: BTreeMap::new(),
        })
        .unwrap();
    let h = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    assert_eq!(
        events
            .iter()
            .filter(|e| event_type(e) == "watch.fired")
            .count(),
        2,
        "both programs must fire"
    );
}
