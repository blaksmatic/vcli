//! Action arbitration. Spec §403.

use vcli_core::program::Priority;
use vcli_core::ProgramId;

/// One pending action for the arbiter to consider.
#[derive(Debug, Clone)]
pub struct Candidate<T> {
    /// Source program.
    pub program_id: ProgramId,
    /// Static priority from `program.priority`.
    pub priority: Priority,
    /// The action / step payload (opaque to the arbiter).
    pub payload: T,
}

/// Outcome of arbitration for one candidate.
#[derive(Debug, Clone)]
pub struct Decision<T> {
    /// Source program.
    pub program_id: ProgramId,
    /// Payload (passed through unchanged).
    pub payload: T,
    /// True if this payload should dispatch.
    pub dispatch: bool,
    /// If not dispatched, the conflicting winner's id.
    pub loser_of: Option<ProgramId>,
}

/// Resolve conflicts: at most one dispatch per tick. Ordering:
/// 1. highest `Priority` wins
/// 2. tiebreak by lexicographic `ProgramId` (larger wins, for determinism)
///
/// If `candidates.len() <= 1` everyone dispatches.
pub fn resolve<T: Clone>(candidates: Vec<Candidate<T>>) -> Vec<Decision<T>> {
    if candidates.len() <= 1 {
        return candidates
            .into_iter()
            .map(|c| Decision {
                program_id: c.program_id,
                payload: c.payload,
                dispatch: true,
                loser_of: None,
            })
            .collect();
    }
    let winner = candidates
        .iter()
        .max_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.program_id.to_string().cmp(&b.program_id.to_string()))
        })
        .map(|c| c.program_id)
        .expect("len > 1 handled above");
    candidates
        .into_iter()
        .map(|c| {
            let is_winner = c.program_id == winner;
            Decision {
                program_id: c.program_id,
                payload: c.payload,
                dispatch: is_winner,
                loser_of: if is_winner { None } else { Some(winner) },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ProgramId {
        let s = format!("{n:02x}345678-1234-4567-8910-111213141516");
        s.parse().unwrap()
    }

    #[test]
    fn single_candidate_always_dispatches() {
        let out = resolve(vec![Candidate {
            program_id: id(1),
            priority: Priority(0),
            payload: "a",
        }]);
        assert_eq!(out.len(), 1);
        assert!(out[0].dispatch);
        assert!(out[0].loser_of.is_none());
    }

    #[test]
    fn higher_priority_wins() {
        let out = resolve(vec![
            Candidate {
                program_id: id(1),
                priority: Priority(0),
                payload: "a",
            },
            Candidate {
                program_id: id(2),
                priority: Priority(5),
                payload: "b",
            },
        ]);
        let dispatched: Vec<_> = out.iter().filter(|d| d.dispatch).collect();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0].program_id, id(2));
        assert_eq!(
            out.iter().find(|d| !d.dispatch).unwrap().loser_of,
            Some(id(2))
        );
    }

    #[test]
    fn tie_breaks_by_program_id_lex_desc() {
        let out = resolve(vec![
            Candidate {
                program_id: id(1),
                priority: Priority(0),
                payload: "a",
            },
            Candidate {
                program_id: id(2),
                priority: Priority(0),
                payload: "b",
            },
        ]);
        let winner = out.iter().find(|d| d.dispatch).unwrap().program_id;
        assert_eq!(winner, id(2));
    }

    #[test]
    fn three_way_drops_two_losers() {
        let out = resolve(vec![
            Candidate {
                program_id: id(1),
                priority: Priority(3),
                payload: "a",
            },
            Candidate {
                program_id: id(2),
                priority: Priority(3),
                payload: "b",
            },
            Candidate {
                program_id: id(3),
                priority: Priority(3),
                payload: "c",
            },
        ]);
        let dispatched: Vec<_> = out.iter().filter(|d| d.dispatch).collect();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0].program_id, id(3));
    }
}
