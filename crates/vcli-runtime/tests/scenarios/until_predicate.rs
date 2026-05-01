//! Scenario: `UntilPredicate` retires the watch when its terminator fires.

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

#[test]
fn until_predicate_retires_watch() {
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();

    let red = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let green = ScriptedCapture::solid(1, 1, [0, 255, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red, green.clone(), green]));
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(0));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert(
        "is_red".into(),
        PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([255, 0, 0]),
            tolerance: 0,
        },
    );
    preds.insert(
        "done".into(),
        PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([0, 255, 0]),
            tolerance: 0,
        },
    );

    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "until".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![Watch {
            when: WatchWhen::ByName("is_red".into()),
            steps: vec![Step::Click {
                at: Target::Absolute(Point { x: 0, y: 0 }),
                button: Button::Left,
            }],
            throttle_ms: 0,
            lifetime: Lifetime::UntilPredicate {
                name: "done".into(),
            },
        }],
        body: vec![],
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
    let fires = events
        .iter()
        .filter(|e| event_type(e) == "watch.fired")
        .count();
    let completes = events
        .iter()
        .filter(|e| event_type(e) == "program.completed")
        .count();
    assert!(
        (1..=2).contains(&fires),
        "expected 1..=2 fires, got {fires}"
    );
    assert_eq!(completes, 1, "expected exactly one completion");
}
