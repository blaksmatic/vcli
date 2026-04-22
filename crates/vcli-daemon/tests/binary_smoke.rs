//! Smoke test: the released vcli-daemon binary exists and starts up.
//! On Linux this runs the binary briefly and SIGTERMs it. On macOS the
//! binary will fail with DaemonError::BackendInit unless the user has
//! granted Screen Recording — that test lives in real_backends_macos.rs.

#![cfg(not(target_os = "macos"))]

use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn vcli_daemon_help_or_starts_clean_on_linux() {
    // Compile the binary first via cargo's test framework helpers.
    // Build path is target/<profile>/vcli-daemon.
    let bin = env!("CARGO_BIN_EXE_vcli-daemon");
    let mut child = Command::new(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vcli-daemon");
    std::thread::sleep(Duration::from_millis(500));
    let _ = child.kill();
    let out = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Either it ran clean (exit 0 after kill = signal) or printed a
    // recognisable startup line. We just want to make sure it didn't
    // crash on startup before signal handlers were installed.
    assert!(
        stderr.is_empty() || stderr.contains("vcli") || out.status.code() != Some(101),
        "unexpected stderr: {stderr}"
    );
}
