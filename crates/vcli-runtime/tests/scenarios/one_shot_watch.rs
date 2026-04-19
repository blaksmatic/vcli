//! Scenario: `Lifetime::OneShot` fires once and the program completes.

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
fn one_shot_watch_fires_once_and_completes() {
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();

    let red_frame = ScriptedCapture::solid(1, 1, [255, 0, 0, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![red_frame]));
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(1_000));

    let id: ProgramId = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut preds = BTreeMap::new();
    preds.insert(
        "red".into(),
        PredicateKind::ColorAt {
            point: Point { x: 0, y: 0 },
            rgb: Rgb([255, 0, 0]),
            tolerance: 0,
        },
    );
    let program = Program {
        version: DslVersion(DslVersion::V0_1.to_string()),
        name: "one_shot".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![Watch {
            when: WatchWhen::ByName("red".into()),
            steps: vec![Step::Click {
                at: Target::Absolute(Point { x: 10, y: 20 }),
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
    };

    let sched = Scheduler::new(
        SchedulerConfig::default(),
        capture,
        input.clone(),
        Perception::new(),
        clock.clone(),
        cmd_rx,
        ev_tx,
    );

    cmd_tx
        .send(SchedulerCommand::SubmitValidated {
            program_id: id,
            program,
        })
        .unwrap();

    let handle = std::thread::spawn(move || sched.run_until_shutdown());
    std::thread::sleep(Duration::from_millis(300));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    handle.join().unwrap();

    let events = drain_events(&ev_rx);
    let types: Vec<_> = events.iter().map(event_type).collect();

    assert!(
        types.iter().any(|t| t == "program.submitted"),
        "types: {types:?}"
    );
    assert!(types.iter().any(|t| t == "watch.fired"), "types: {types:?}");
    assert!(
        types.iter().any(|t| t == "program.completed"),
        "types: {types:?}"
    );

    let clicks: Vec<_> = input
        .calls()
        .into_iter()
        .filter(|c| matches!(c, common::mock_input::Call::Click(_, _)))
        .collect();
    assert_eq!(
        clicks.len(),
        1,
        "expected exactly one click, got {clicks:?}"
    );
}
