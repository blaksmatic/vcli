//! vcli-daemon entrypoint. Minimal argv surface:
//!   * `--version` → print crate version, exit 0
//!   * `--help`    → print usage, exit 0
//!
//! All real work lives in `vcli_daemon::run_foreground`.

use std::process::ExitCode;

use vcli_daemon::{run_foreground, Config, DaemonError, RuntimeBackends};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vcli-daemon {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: vcli-daemon");
        println!("  no flags — runs in the foreground until SIGTERM/SIGINT");
        return ExitCode::SUCCESS;
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("vcli-daemon: failed to build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let cfg = match Config::from_platform_defaults() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("vcli-daemon: {e}");
            return ExitCode::from(1);
        }
    };

    let factory: vcli_daemon::RuntimeFactory = Box::new(default_runtime_factory);

    match rt.block_on(run_foreground(cfg, factory)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(DaemonError::AlreadyRunning { pid, .. }) => {
            eprintln!("vcli-daemon: already running (pid {pid})");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("vcli-daemon: {e}");
            ExitCode::from(1)
        }
    }
}

/// Real-backend factory used in production. For v0 the daemon binary falls
/// back to mock backends on every platform — a future lane wires the real
/// macOS capture + input.
fn default_runtime_factory() -> Result<RuntimeBackends, DaemonError> {
    let capture: Box<dyn vcli_capture::Capture> = Box::new(vcli_capture::MockCapture::empty());
    let input: std::sync::Arc<dyn vcli_input::InputSink> =
        std::sync::Arc::new(vcli_input::MockInputSink::new());
    let perception = vcli_perception::Perception::default();
    let clock: std::sync::Arc<dyn vcli_runtime::RuntimeClock> =
        std::sync::Arc::new(vcli_runtime::SystemRuntimeClock::new());
    Ok(RuntimeBackends {
        capture,
        input,
        perception,
        clock,
    })
}
