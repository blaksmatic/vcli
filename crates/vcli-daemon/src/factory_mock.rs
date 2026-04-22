//! Mock `RuntimeBackends` factory used on non-macOS targets and as a
//! convenient default in dev / CI on macOS when real TCC isn't available.
//!
//! Production macOS daemons should NOT call this — they call the
//! macOS factory in `factory_macos` via `build_default_backends`.

use std::sync::Arc;

use crate::error::DaemonResult;
use crate::run::RuntimeBackends;

/// Build a fully-mocked `RuntimeBackends`. Always succeeds.
///
/// # Errors
///
/// Never fails; the `Result` wrapper preserves the same signature as the
/// real factory so callers can be swapped without type changes.
pub fn build() -> DaemonResult<RuntimeBackends> {
    Ok(RuntimeBackends {
        capture: Box::new(vcli_capture::MockCapture::empty()),
        input: Arc::new(vcli_input::MockInputSink::new()),
        perception: vcli_perception::Perception::default(),
        clock: Arc::new(vcli_runtime::SystemRuntimeClock::new()),
        _shutdown_guard: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_returns_ok_with_no_shutdown_guard() {
        let b = build().expect("mock factory cannot fail");
        // Why: field is intentionally underscore-prefixed (pub API decision in
        // run.rs); we need to read it here to assert the mock leaves it None.
        #[allow(clippy::used_underscore_binding)]
        let guard_is_none = b._shutdown_guard.is_none();
        assert!(guard_is_none);
    }
}
