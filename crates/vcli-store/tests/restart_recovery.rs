//! Integration: `Store::open` transitions any row left in `running` to
//! `failed(daemon_restart)` and returns the recovered programs with their
//! `body_cursor` preserved. Spec §Restart semantics, step 4.

use tempfile::tempdir;

use vcli_core::ids::ProgramId;
use vcli_core::state::ProgramState;
use vcli_store::{NewProgram, Store};

fn insert_running(store: &mut Store, id: ProgramId, cursor: u32) {
    store
        .insert_program(&NewProgram {
            id,
            name: "x",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
    store.update_state(id, ProgramState::Running, 100).unwrap();
    store.set_body_cursor(id, cursor).unwrap();
}

#[test]
fn running_programs_transition_to_failed_on_reopen() {
    let d = tempdir().unwrap();
    let id1 = ProgramId::new();
    let id2 = ProgramId::new();

    {
        let (mut s, _) = Store::open(d.path()).unwrap();
        insert_running(&mut s, id1, 3);
        insert_running(&mut s, id2, 0);
    }

    // Simulate a daemon restart: reopen the store.
    let (s, recovered) = Store::open(d.path()).unwrap();
    assert_eq!(recovered.len(), 2);

    let r1 = s.get_program(id1).unwrap();
    assert_eq!(r1.state, ProgramState::Failed);
    assert_eq!(r1.last_error_code.as_deref(), Some("daemon_restart"));
    assert_eq!(r1.body_cursor, 3); // cursor preserved

    let r2 = s.get_program(id2).unwrap();
    assert_eq!(r2.state, ProgramState::Failed);
    assert_eq!(r2.body_cursor, 0);
}

#[test]
fn second_reopen_after_recovery_is_a_no_op() {
    let d = tempdir().unwrap();
    let id = ProgramId::new();
    {
        let (mut s, _) = Store::open(d.path()).unwrap();
        insert_running(&mut s, id, 5);
    }
    let (_, recovered_first) = Store::open(d.path()).unwrap();
    assert_eq!(recovered_first.len(), 1);

    let (_, recovered_second) = Store::open(d.path()).unwrap();
    assert!(recovered_second.is_empty(), "no rows still in running");
}
