//! Legal program-state transitions. Spec §370.

use vcli_core::state::ProgramState;

/// Why a transition is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// The `from`→`to` pair is not allowed (e.g. re-entering `Running` from
    /// `Completed`).
    Illegal {
        /// Prior state.
        from: ProgramState,
        /// Attempted state.
        to: ProgramState,
    },
}

/// Return `Ok(())` if `from → to` is legal per spec §370, else
/// `Err(TransitionError::Illegal)`.
///
/// # Errors
///
/// Returns `Illegal` when the transition isn't in the allowed set.
pub fn validate(from: ProgramState, to: ProgramState) -> Result<(), TransitionError> {
    use ProgramState::{Blocked, Cancelled, Completed, Failed, Pending, Running, Waiting};
    let ok = matches!(
        (from, to),
        (Pending, Waiting)
            | (Pending, Cancelled)
            | (Waiting, Running)
            | (Waiting, Cancelled)
            | (Waiting, Failed)
            | (Running, Blocked)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, Cancelled)
            | (Blocked, Running)
            | (Blocked, Cancelled)
            | (Blocked, Failed)
            | (Failed, Running)
    );
    if ok {
        Ok(())
    } else {
        Err(TransitionError::Illegal { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProgramState::{Cancelled, Completed, Failed, Pending, Running, Waiting};

    #[test]
    fn happy_path_transitions_are_legal() {
        for (f, t) in [
            (Pending, Waiting),
            (Waiting, Running),
            (Running, Completed),
            (Running, Failed),
            (Running, Cancelled),
            (Failed, Running),
        ] {
            validate(f, t).unwrap_or_else(|_| panic!("{f:?}→{t:?} should be legal"));
        }
    }

    #[test]
    fn terminal_states_cannot_restart_implicitly() {
        assert!(validate(Completed, Running).is_err());
        assert!(validate(Cancelled, Running).is_err());
    }

    #[test]
    fn waiting_to_completed_is_illegal() {
        assert!(validate(Waiting, Completed).is_err());
    }
}
