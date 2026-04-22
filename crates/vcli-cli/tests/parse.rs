//! Parse-coverage for `vcli`. Does not connect to a daemon; only checks that
//! clap grammar matches spec §567–590.

use clap::Parser;

use vcli_cli::{Cli, Command, DaemonCommand, StateFilter};

fn ok(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).unwrap()
}

fn err(args: &[&str]) {
    assert!(
        Cli::try_parse_from(args).is_err(),
        "expected err from {args:?}"
    );
}

#[test]
fn top_level_help_does_not_require_subcommand_to_show() {
    let r = Cli::try_parse_from(["vcli", "--help"]);
    assert!(r.is_err());
    assert_eq!(r.unwrap_err().kind(), clap::error::ErrorKind::DisplayHelp);
}

#[test]
fn every_v0_command_parses_minimally() {
    let id = "12345678-1234-4567-8910-111213141516";
    ok(&["vcli", "submit", "p.json"]);
    ok(&["vcli", "list"]);
    ok(&["vcli", "cancel", id]);
    ok(&["vcli", "logs", id]);
    ok(&["vcli", "resume", id]);
    ok(&["vcli", "daemon", "start"]);
    ok(&["vcli", "daemon", "run"]);
    ok(&["vcli", "daemon", "stop"]);
    ok(&["vcli", "daemon", "status"]);
    ok(&["vcli", "health"]);
    ok(&["vcli", "gc"]);
}

#[test]
fn json_flag_is_global() {
    for args in [
        &["vcli", "--json", "list"][..],
        &["vcli", "list", "--json"][..],
        &["vcli", "--json", "submit", "p.json"][..],
        &["vcli", "submit", "p.json", "--json"][..],
    ] {
        let c = ok(args);
        assert!(c.json, "{args:?}");
    }
}

#[test]
fn socket_flag_is_global() {
    let c = ok(&["vcli", "--socket", "/tmp/x.sock", "health"]);
    assert_eq!(
        c.socket.as_deref().unwrap().to_str().unwrap(),
        "/tmp/x.sock"
    );
    let c = ok(&["vcli", "health", "--socket", "/tmp/y.sock"]);
    assert_eq!(
        c.socket.as_deref().unwrap().to_str().unwrap(),
        "/tmp/y.sock"
    );
}

#[test]
fn list_state_filter_rejects_unknown_value() {
    err(&["vcli", "list", "--state", "frobnicating"]);
}

#[test]
fn list_state_filter_accepts_each_known_value() {
    for (arg, want) in [
        ("waiting", StateFilter::Waiting),
        ("running", StateFilter::Running),
        ("completed", StateFilter::Completed),
        ("failed", StateFilter::Failed),
        ("cancelled", StateFilter::Cancelled),
    ] {
        let c = ok(&["vcli", "list", "--state", arg]);
        match c.command {
            Command::List(a) => assert_eq!(a.state, Some(want)),
            _ => panic!(),
        }
    }
}

#[test]
fn cancel_rejects_non_uuid() {
    err(&["vcli", "cancel", "not-a-uuid"]);
}

#[test]
fn submit_without_file_errors() {
    err(&["vcli", "submit"]);
}

#[test]
fn daemon_without_subcommand_errors() {
    err(&["vcli", "daemon"]);
}

#[test]
fn daemon_status_parses() {
    let c = ok(&["vcli", "daemon", "status"]);
    match c.command {
        Command::Daemon(DaemonCommand::Status) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn resume_from_start_flag_can_be_before_or_after_id() {
    let id = "12345678-1234-4567-8910-111213141516";
    ok(&["vcli", "resume", id, "--from-start"]);
    ok(&["vcli", "resume", "--from-start", id]);
}
