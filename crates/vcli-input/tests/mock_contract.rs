//! Contract tests that run against [`MockInputSink`] and also serve as a
//! template for asserting any future `InputSink` impl.

use std::time::Duration;

use vcli_core::action::{Button, InputAction, Modifier};
use vcli_core::geom::Point;

use vcli_input::error::InputError;
use vcli_input::kill_switch::KillSwitch;
use vcli_input::mock::{MockCall, MockInputSink};
use vcli_input::sink::{DragSegment, InputSink};

fn new_mock() -> MockInputSink {
    MockInputSink::new()
}

#[test]
fn mouse_move_records_once() {
    let m = new_mock();
    m.mouse_move(Point { x: 100, y: 200 }).unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::Action(InputAction::Move {
            at: Point { x: 100, y: 200 }
        })]
    );
}

#[test]
fn click_preserves_modifiers_and_hold() {
    let m = new_mock();
    m.click(
        Point { x: 5, y: 6 },
        Button::Right,
        &[Modifier::Cmd, Modifier::Ctrl],
        75,
    )
    .unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::ClickDetailed {
            at: Point { x: 5, y: 6 },
            button: Button::Right,
            modifiers: vec![Modifier::Cmd, Modifier::Ctrl],
            hold_ms: 75,
        }]
    );
}

#[test]
fn double_click_emits_distinct_variant() {
    let m = new_mock();
    m.double_click(Point { x: 1, y: 1 }, Button::Left).unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::DoubleClick {
            at: Point { x: 1, y: 1 },
            button: Button::Left
        }]
    );
}

#[test]
fn drag_with_multiple_segments_records_all_endpoints() {
    let m = new_mock();
    m.drag(
        Point { x: 0, y: 0 },
        &[
            DragSegment {
                to: Point { x: 10, y: 10 },
                duration: Duration::from_millis(5),
            },
            DragSegment {
                to: Point { x: 20, y: 20 },
                duration: Duration::from_millis(5),
            },
        ],
        Button::Left,
    )
    .unwrap();
    match &m.calls()[0] {
        MockCall::Drag { from, to, button } => {
            assert_eq!(*from, Point { x: 0, y: 0 });
            assert_eq!(to, &vec![Point { x: 10, y: 10 }, Point { x: 20, y: 20 }]);
            assert_eq!(*button, Button::Left);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn type_text_records_input_action() {
    let m = new_mock();
    m.type_text("hello 世界").unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::Action(InputAction::Type {
            text: "hello 世界".into()
        })]
    );
}

#[test]
fn key_combo_records_modifiers() {
    let m = new_mock();
    m.key_combo(&[Modifier::Cmd, Modifier::Shift], "s").unwrap();
    assert_eq!(
        m.calls(),
        vec![MockCall::Action(InputAction::Key {
            key: "s".into(),
            modifiers: vec![Modifier::Cmd, Modifier::Shift],
        })]
    );
}

#[test]
fn empty_drag_is_rejected() {
    let m = new_mock();
    let e = m.drag(Point { x: 0, y: 0 }, &[], Button::Left).unwrap_err();
    assert!(matches!(e, InputError::InvalidArgument(_)));
}

#[test]
fn forced_error_bubbles_as_backend_failure() {
    let m = MockInputSink::new();
    m.fail_with("os returned -1");
    let e = m.type_text("nope").unwrap_err();
    assert!(matches!(e, InputError::Backend { .. }));
}

#[test]
fn kill_switch_engaged_halts_every_method() {
    let kill = KillSwitch::new();
    let m = MockInputSink::with_kill_switch(kill.clone());
    kill.engage();
    assert!(matches!(
        m.mouse_move(Point { x: 0, y: 0 }).unwrap_err(),
        InputError::Halted
    ));
    assert!(matches!(m.type_text("x").unwrap_err(), InputError::Halted));
    assert!(matches!(
        m.key_combo(&[], "a").unwrap_err(),
        InputError::Halted
    ));
}
