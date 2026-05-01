//! End-to-end: real IpcServer + FakeHandler, exercised through each command's
//! public `run()` signature.

mod common;

use std::path::PathBuf;

use vcli_cli::cli::{CancelArgs, ListArgs, LogsArgs, OutputMode, ResumeArgs, SubmitArgs};
use vcli_cli::{commands, error::CliError};

use common::FakeDaemon;

#[tokio::test]
async fn health_command_prints_ok() {
    let d = FakeDaemon::start().await;
    let s = commands::health::run(&d.socket, OutputMode::Pretty)
        .await
        .unwrap();
    assert!(s.contains("daemon: ok"), "{s}");
}

#[tokio::test]
async fn health_json_mode_returns_valid_json() {
    let d = FakeDaemon::start().await;
    let s = commands::health::run(&d.socket, OutputMode::Json)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert!(v.is_object(), "{v}");
}

#[tokio::test]
async fn gc_command_reports_ok() {
    let d = FakeDaemon::start().await;
    let s = commands::gc::run(&d.socket, OutputMode::Pretty)
        .await
        .unwrap();
    assert!(s.starts_with("gc: ok"), "{s}");
}

#[tokio::test]
async fn list_command_renders_empty_table() {
    let d = FakeDaemon::start().await;
    let s = commands::list::run(&d.socket, OutputMode::Pretty, &ListArgs { state: None })
        .await
        .unwrap();
    assert!(s.contains("id"), "{s}");
    assert_eq!(s.lines().count(), 1);
}

#[tokio::test]
async fn cancel_command_roundtrips() {
    let d = FakeDaemon::start().await;
    let id = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let s = commands::cancel::run(
        &d.socket,
        OutputMode::Pretty,
        &CancelArgs {
            program_id: id,
            reason: None,
        },
    )
    .await
    .unwrap();
    assert!(s.contains("cancelled"), "{s}");
}

#[tokio::test]
async fn resume_command_roundtrips() {
    let d = FakeDaemon::start().await;
    let id = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let s = commands::resume::run(
        &d.socket,
        OutputMode::Pretty,
        &ResumeArgs {
            program_id: id,
            from_start: true,
        },
    )
    .await
    .unwrap();
    assert!(s.starts_with("resumed"), "{s}");
}

#[tokio::test]
async fn logs_command_streams_events_and_exits() {
    let d = FakeDaemon::start().await;
    let id = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut buf: Vec<u8> = Vec::new();
    commands::logs::run(
        &d.socket,
        OutputMode::Pretty,
        &LogsArgs {
            program_id: id,
            follow: false,
            since: None,
        },
        &mut buf,
    )
    .await
    .unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("daemon.started"), "{out}");
    assert!(out.contains("daemon.stopped"), "{out}");
}

#[tokio::test]
async fn logs_command_json_mode_emits_one_json_per_line() {
    let d = FakeDaemon::start().await;
    let id = "12345678-1234-4567-8910-111213141516".parse().unwrap();
    let mut buf: Vec<u8> = Vec::new();
    commands::logs::run(
        &d.socket,
        OutputMode::Json,
        &LogsArgs {
            program_id: id,
            follow: false,
            since: None,
        },
        &mut buf,
    )
    .await
    .unwrap();
    let out = String::from_utf8(buf).unwrap();
    let lines: Vec<_> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let _: serde_json::Value = serde_json::from_str(line).unwrap();
    }
}

#[tokio::test]
async fn submit_command_writes_program_id() {
    let d = FakeDaemon::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("p.json");
    std::fs::write(
        &file,
        r#"{
            "version": "0.1",
            "name": "t",
            "trigger": {"kind":"on_submit"},
            "predicates": {},
            "watches": [],
            "body": []
        }"#,
    )
    .unwrap();
    let mut buf: Vec<u8> = Vec::new();
    commands::submit::run(
        &d.socket,
        OutputMode::Pretty,
        &SubmitArgs { file, watch: false },
        &mut buf,
    )
    .await
    .unwrap();
    let out = String::from_utf8(buf).unwrap();
    let first_line = out.lines().next().unwrap().trim();
    assert_eq!(first_line.len(), 36, "{out}");
    assert_eq!(first_line.matches('-').count(), 4);
}

#[tokio::test]
async fn submit_command_sends_file_parent_as_base_dir() {
    let d = FakeDaemon::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("p.json");
    std::fs::write(
        &file,
        r#"{
            "version": "0.1",
            "name": "t",
            "trigger": {"kind":"on_submit"},
            "predicates": {},
            "watches": [],
            "body": []
        }"#,
    )
    .unwrap();
    let mut buf: Vec<u8> = Vec::new();
    commands::submit::run(
        &d.socket,
        OutputMode::Pretty,
        &SubmitArgs {
            file: file.clone(),
            watch: false,
        },
        &mut buf,
    )
    .await
    .unwrap();

    let seen = d.handler.received.lock().await;
    let Some(vcli_ipc::RequestOp::Submit { base_dir, .. }) = seen.last() else {
        panic!("expected submit op, got {seen:?}");
    };
    let expected = std::fs::canonicalize(&file)
        .unwrap()
        .parent()
        .unwrap()
        .display()
        .to_string();
    assert_eq!(base_dir.as_deref(), Some(expected.as_str()));
}

#[tokio::test]
async fn submit_with_invalid_json_file_is_validation_error() {
    let d = FakeDaemon::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("bad.json");
    std::fs::write(&file, "{not json").unwrap();
    let mut buf: Vec<u8> = Vec::new();
    let err = commands::submit::run(
        &d.socket,
        OutputMode::Pretty,
        &SubmitArgs { file, watch: false },
        &mut buf,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CliError::Validation(_)));
}

#[tokio::test]
async fn submit_with_dsl_rejected_program_is_validation_error() {
    let d = FakeDaemon::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("cycle.json");
    std::fs::write(
        &file,
        r#"{
            "version": "0.1",
            "name": "p",
            "trigger": {"kind":"on_submit"},
            "predicates": {
                "a": {"kind":"not","of":"b"},
                "b": {"kind":"not","of":"a"}
            },
            "watches": [],
            "body": []
        }"#,
    )
    .unwrap();
    let mut buf: Vec<u8> = Vec::new();
    let err = commands::submit::run(
        &d.socket,
        OutputMode::Pretty,
        &SubmitArgs { file, watch: false },
        &mut buf,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CliError::Validation(_)));
}

#[tokio::test]
async fn daemon_status_reports_running_against_fake_daemon() {
    use vcli_cli::cli::DaemonCommand;
    let d = FakeDaemon::start().await;
    let mut buf: Vec<u8> = Vec::new();
    commands::daemon::run(
        &d.socket,
        OutputMode::Pretty,
        &DaemonCommand::Status,
        &mut buf,
    )
    .await
    .unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("running"), "{out}");
}

#[tokio::test]
async fn daemon_status_without_socket_reports_not_running() {
    use vcli_cli::cli::DaemonCommand;
    let bogus = PathBuf::from("/tmp/vcli-definitely-does-not-exist.sock");
    let mut buf: Vec<u8> = Vec::new();
    commands::daemon::run(&bogus, OutputMode::Pretty, &DaemonCommand::Status, &mut buf)
        .await
        .unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("not running"), "{out}");
}

#[tokio::test]
async fn missing_daemon_yields_daemon_down_error() {
    let bogus = PathBuf::from("/tmp/vcli-definitely-does-not-exist.sock");
    let err = commands::health::run(&bogus, OutputMode::Pretty)
        .await
        .unwrap_err();
    assert!(matches!(err, CliError::DaemonDown(_)));
    assert_eq!(err.exit_code(), vcli_cli::error::ExitCode::DaemonDown);
}
