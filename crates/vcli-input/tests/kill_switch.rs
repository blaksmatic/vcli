//! End-to-end behavior: engaging the kill switch makes every subsequent call
//! fail with `Halted` and stops recording. Previously recorded calls stay.

use std::time::Duration;

use vcli_core::action::{Button, Modifier};
use vcli_core::geom::Point;

use vcli_input::kill_switch::KillSwitch;
use vcli_input::mock::{MockCall, MockInputSink};
use vcli_input::sink::{DragSegment, InputSink};
use vcli_input::InputError;

#[test]
fn engage_halts_every_subsequent_method() {
    let kill = KillSwitch::new();
    let mock = MockInputSink::with_kill_switch(kill.clone());

    // Before engage: all methods record.
    mock.mouse_move(Point { x: 1, y: 1 }).unwrap();
    mock.click(Point { x: 1, y: 1 }, Button::Left, &[], 0)
        .unwrap();
    mock.double_click(Point { x: 2, y: 2 }, Button::Left)
        .unwrap();
    mock.drag(
        Point { x: 0, y: 0 },
        &[DragSegment {
            to: Point { x: 5, y: 5 },
            duration: Duration::from_millis(5),
        }],
        Button::Left,
    )
    .unwrap();
    mock.type_text("hello").unwrap();
    mock.key_combo(&[Modifier::Cmd], "s").unwrap();
    assert_eq!(mock.calls().len(), 6);

    // Engage.
    kill.engage();

    for result in [
        mock.mouse_move(Point { x: 0, y: 0 }),
        mock.click(Point { x: 0, y: 0 }, Button::Left, &[], 0),
        mock.double_click(Point { x: 0, y: 0 }, Button::Left),
        mock.drag(
            Point { x: 0, y: 0 },
            &[DragSegment {
                to: Point { x: 1, y: 1 },
                duration: Duration::from_millis(1),
            }],
            Button::Left,
        ),
        mock.type_text("nope"),
        mock.key_combo(&[], "a"),
    ] {
        let e = result.unwrap_err();
        assert!(matches!(e, InputError::Halted), "got {e:?}");
    }

    // Recorded calls are unchanged (no new ones added after engage).
    assert_eq!(mock.calls().len(), 6);
}

#[test]
fn observer_wakes_immediately_when_switch_engaged_concurrently() {
    let kill = KillSwitch::new();
    let obs = kill.subscribe();
    let k2 = kill.clone();
    let h = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        k2.engage();
    });
    assert!(obs.wait_until_engaged(Duration::from_secs(1), Duration::from_millis(2)));
    h.join().unwrap();
}

#[test]
fn disengage_reenables_calls() {
    let kill = KillSwitch::new();
    let mock = MockInputSink::with_kill_switch(kill.clone());
    kill.engage();
    assert!(matches!(
        mock.mouse_move(Point { x: 1, y: 1 }).unwrap_err(),
        InputError::Halted
    ));
    kill.disengage();
    mock.mouse_move(Point { x: 2, y: 2 }).unwrap();
    assert_eq!(
        mock.calls(),
        vec![MockCall::Action(vcli_core::action::InputAction::Move {
            at: Point { x: 2, y: 2 }
        })]
    );
}
