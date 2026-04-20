//! Smoke test: invoke the built `vcli` binary as a subprocess.

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_exits_zero() {
    Command::cargo_bin("vcli")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("vcli"));
}

#[test]
fn version_exits_zero() {
    Command::cargo_bin("vcli")
        .unwrap()
        .arg("--version")
        .assert()
        .success();
}

#[test]
fn health_without_daemon_exits_4() {
    let tmp = tempfile::tempdir().unwrap();
    let sock = tmp.path().join("vcli.sock");
    Command::cargo_bin("vcli")
        .unwrap()
        .args(["--socket", sock.to_str().unwrap(), "health"])
        .assert()
        .code(4);
}

#[test]
fn cancel_with_garbage_id_is_nonzero() {
    Command::cargo_bin("vcli")
        .unwrap()
        .args(["cancel", "not-a-uuid"])
        .assert()
        .failure();
}

#[test]
fn gc_without_daemon_exits_4() {
    let tmp = tempfile::tempdir().unwrap();
    let sock = tmp.path().join("vcli.sock");
    Command::cargo_bin("vcli")
        .unwrap()
        .args(["--socket", sock.to_str().unwrap(), "gc"])
        .assert()
        .code(4);
}

#[test]
fn daemon_status_without_socket_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let sock = tmp.path().join("vcli.sock");
    Command::cargo_bin("vcli")
        .unwrap()
        .args(["--socket", sock.to_str().unwrap(), "daemon", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not running"));
}
