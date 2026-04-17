//! Integration: the 7-day retention boundary keeps recent terminal programs
//! and prunes older ones.

use tempfile::tempdir;
use vcli_core::ids::ProgramId;
use vcli_core::state::ProgramState;
use vcli_store::{NewProgram, Store, RETENTION_DAYS};

const MS_PER_DAY: i64 = 86_400_000;

#[test]
fn seven_day_retention_boundary() {
    let d = tempdir().unwrap();
    let (mut s, _) = Store::open(d.path()).unwrap();
    let now = 1_700_000_000_000_i64;
    let cutoff = now - i64::from(RETENTION_DAYS) * MS_PER_DAY;

    let a = ProgramId::new();
    let b = ProgramId::new();
    for (id, finished) in [(a, cutoff - 1), (b, cutoff + 1)] {
        s.insert_program(&NewProgram {
            id,
            name: "x",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: 0,
            labels_json: "{}",
        })
        .unwrap();
        s.update_state(id, ProgramState::Completed, finished)
            .unwrap();
    }

    let n = s.gc_programs(cutoff).unwrap();
    assert_eq!(n, 1);
    assert!(s.get_program(a).is_err());
    assert!(s.get_program(b).is_ok());
}
