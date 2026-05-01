//! Scenario: `Lifetime::TimeoutMs` retires the watch once the window elapses
//! even without a rising edge.

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use vcli_core::geom::Point;
use vcli_core::predicate::{PredicateKind, Rgb};
use vcli_core::watch::{Lifetime, Watch, WatchWhen};
use vcli_core::{program::DslVersion, trigger::Trigger, Program};
use vcli_perception::Perception;
use vcli_runtime::clock::ManualClock;

#[test]
fn watch_timeout_retires_after_budget() {
    let (cmd_tx, cmd_rx) = unbounded::<SchedulerCommand>();
    let (ev_tx, ev_rx) = unbounded::<Event>();
    let blue = ScriptedCapture::solid(1, 1, [0, 0, 255, 255]);
    let capture = Box::new(ScriptedCapture::new(vec![blue]));
    let input = Arc::new(RecordingInputSink::new());
    let clock = Arc::new(ManualClock::new(0));

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
        name: "timeout".into(),
        id: None,
        trigger: Trigger::OnSubmit,
        predicates: preds,
        watches: vec![Watch {
            when: WatchWhen::ByName("red".into()),
            steps: vec![],
            throttle_ms: 0,
            lifetime: Lifetime::TimeoutMs { ms: 200 },
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
        input,
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
    std::thread::sleep(Duration::from_millis(500));
    cmd_tx.send(SchedulerCommand::Shutdown).unwrap();
    h.join().unwrap();

    let events = drain_events(&ev_rx);
    assert_eq!(
        events
            .iter()
            .filter(|e| event_type(e) == "watch.fired")
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| event_type(e) == "program.completed")
            .count(),
        1
    );
}
