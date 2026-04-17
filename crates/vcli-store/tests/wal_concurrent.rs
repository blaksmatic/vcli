//! Integration: SQLite WAL mode permits concurrent readers during writes.
//! Opens one writer + two readers in separate threads and asserts no
//! `SQLITE_BUSY` failures within the 5s busy timeout.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rusqlite::Connection;
use tempfile::tempdir;
use vcli_core::ids::ProgramId;
use vcli_core::state::ProgramState;
use vcli_store::{paths::db_path, pragmas::apply_pragmas, NewProgram, Store};

#[test]
fn two_readers_coexist_with_one_writer() {
    let d = tempdir().unwrap();
    let (mut s, _) = Store::open(d.path()).unwrap();
    // Seed enough rows for readers to iterate.
    for i in 0..50 {
        let id = ProgramId::new();
        s.insert_program(&NewProgram {
            id,
            name: "seed",
            source_json: "{}",
            state: ProgramState::Pending,
            submitted_at: i,
            labels_json: "{}",
        })
        .unwrap();
    }

    let path = Arc::new(db_path(d.path()));

    // Reader thread helper.
    let mk_reader = |iters: usize| {
        let path = path.clone();
        thread::spawn(move || {
            let c = Connection::open(&*path).unwrap();
            apply_pragmas(&c).unwrap();
            for _ in 0..iters {
                let n: i64 = c
                    .query_row("SELECT COUNT(*) FROM programs", [], |r| r.get(0))
                    .unwrap();
                assert!(n >= 50);
                thread::sleep(Duration::from_millis(2));
            }
        })
    };

    let r1 = mk_reader(20);
    let r2 = mk_reader(20);

    // Writer does inserts concurrently.
    let w = {
        let path = path.clone();
        thread::spawn(move || {
            let c = Connection::open(&*path).unwrap();
            apply_pragmas(&c).unwrap();
            for i in 0..20 {
                c.execute(
                    "INSERT INTO programs
                       (id, name, source_json, state, submitted_at, labels_json, body_cursor)
                     VALUES (?1, 'w', '{}', 'pending', ?2, '{}', 0)",
                    rusqlite::params![ProgramId::new().to_string(), 1000 + i],
                )
                .unwrap();
            }
        })
    };

    r1.join().unwrap();
    r2.join().unwrap();
    w.join().unwrap();

    // Final count = 50 seed + 20 inserts = 70.
    let (s, _) = Store::open(d.path()).unwrap();
    let n: i64 = s
        .conn()
        .query_row("SELECT COUNT(*) FROM programs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 70);
}
