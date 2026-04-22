//! Clap derive definitions. Pure data; execution lives in `crate::commands`.
//! Spec §567–590 is authoritative for every flag.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use vcli_core::ProgramId;

/// Top-level `vcli` invocation.
#[derive(Debug, Parser)]
#[command(
    name = "vcli",
    version,
    about = "vcli — reactive screen runtime control plane",
    long_about = None,
)]
pub struct Cli {
    /// Override socket path (default: `$VCLI_SOCKET` → macOS `$TMPDIR/vcli-$UID.sock`
    /// → Linux `$XDG_RUNTIME_DIR/vcli.sock` → `/tmp/vcli-$UID.sock`).
    #[arg(long, global = true, value_name = "PATH")]
    pub socket: Option<PathBuf>,

    /// Emit machine-readable JSON. Per spec §589: every command supports this.
    #[arg(long, global = true)]
    pub json: bool,

    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Which output mode is requested.
    #[must_use]
    pub fn output_mode(&self) -> OutputMode {
        if self.json {
            OutputMode::Json
        } else {
            OutputMode::Pretty
        }
    }
}

/// Output rendering backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable tables / single-line summaries.
    Pretty,
    /// One JSON object per invocation (or one per line on streams).
    Json,
}

/// Subcommands. Spec §567–590.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate locally, submit to the daemon, print the assigned program id.
    Submit(SubmitArgs),

    /// List known programs, optionally filtered by state.
    List(ListArgs),

    /// Cancel a running program.
    Cancel(CancelArgs),

    /// Stream program-scoped events.
    Logs(LogsArgs),

    /// Resume a program that failed with `daemon_restart`.
    Resume(ResumeArgs),

    /// Daemon lifecycle (start / run / stop / status).
    #[command(subcommand)]
    Daemon(DaemonCommand),

    /// Print daemon version, uptime, cache sizes.
    Health,

    /// Force a GC cycle over the asset store.
    Gc,
}

/// `vcli submit <file.json> [--watch]`
#[derive(Debug, Args)]
pub struct SubmitArgs {
    /// Path to the program JSON file.
    pub file: PathBuf,

    /// After submission, stream the program's events until it terminates.
    #[arg(long)]
    pub watch: bool,
}

/// State filter for `list`. Values match `vcli_core::ProgramState::as_str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum StateFilter {
    /// Waiting for trigger.
    Waiting,
    /// Running (trigger fired).
    Running,
    /// Completed successfully.
    Completed,
    /// Failed.
    Failed,
    /// Cancelled.
    Cancelled,
}

impl StateFilter {
    /// Wire-string form (passed to the daemon in `RequestOp::List.state`).
    #[must_use]
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// `vcli list [--state STATE]`
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Filter by program state.
    #[arg(long)]
    pub state: Option<StateFilter>,
}

/// `vcli cancel <id> [--reason TEXT]`
#[derive(Debug, Args)]
pub struct CancelArgs {
    /// Program id (UUID).
    pub program_id: ProgramId,

    /// Optional human-readable reason (informational only; daemon echoes it).
    #[arg(long)]
    pub reason: Option<String>,
}

/// `vcli logs <id> [--follow] [--since ISO8601]`
#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Program id.
    pub program_id: ProgramId,

    /// Keep the stream open and push future events.
    #[arg(long)]
    pub follow: bool,

    /// Drop events strictly older than this timestamp (ISO 8601).
    #[arg(long, value_name = "ISO8601")]
    pub since: Option<String>,
}

/// `vcli resume <id> [--from-start]`
#[derive(Debug, Args)]
pub struct ResumeArgs {
    /// Program id.
    pub program_id: ProgramId,

    /// Ignore `body_cursor` and restart from step 0.
    #[arg(long)]
    pub from_start: bool,
}

/// `vcli daemon { start | run | stop | status }`
#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    /// Fork + detach a `vcli-daemon` child. Idempotent.
    Start,
    /// Foreground daemon for launchd/systemd. Execs `vcli-daemon` directly.
    Run,
    /// Send `Shutdown` and wait for the socket to disappear.
    Stop,
    /// Print running/not-running plus pid + socket path.
    Status,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn submit_without_file_errors() {
        let r = Cli::try_parse_from(["vcli", "submit"]);
        assert!(r.is_err());
    }

    #[test]
    fn submit_with_file_parses() {
        let c = parse(&["vcli", "submit", "program.json"]);
        match c.command {
            Command::Submit(a) => {
                assert_eq!(a.file, PathBuf::from("program.json"));
                assert!(!a.watch);
            }
            _ => panic!("wrong command"),
        }
        assert!(!c.json);
        assert!(c.socket.is_none());
    }

    #[test]
    fn submit_watch_and_json_flags_parse() {
        let c = parse(&["vcli", "--json", "submit", "p.json", "--watch"]);
        assert!(c.json);
        assert_eq!(c.output_mode(), OutputMode::Json);
        match c.command {
            Command::Submit(a) => assert!(a.watch),
            _ => panic!(),
        }
    }

    #[test]
    fn global_socket_override_parses() {
        let c = parse(&["vcli", "--socket", "/tmp/x.sock", "health"]);
        assert_eq!(c.socket, Some(PathBuf::from("/tmp/x.sock")));
        assert!(matches!(c.command, Command::Health));
    }

    #[test]
    fn list_without_state_filter() {
        let c = parse(&["vcli", "list"]);
        match c.command {
            Command::List(a) => assert!(a.state.is_none()),
            _ => panic!(),
        }
    }

    #[test]
    fn list_with_each_state_value() {
        for (arg, want) in [
            ("running", StateFilter::Running),
            ("waiting", StateFilter::Waiting),
            ("completed", StateFilter::Completed),
            ("failed", StateFilter::Failed),
            ("cancelled", StateFilter::Cancelled),
        ] {
            let c = parse(&["vcli", "list", "--state", arg]);
            match c.command {
                Command::List(a) => assert_eq!(a.state, Some(want)),
                _ => panic!("{arg}"),
            }
        }
    }

    #[test]
    fn list_rejects_unknown_state() {
        let r = Cli::try_parse_from(["vcli", "list", "--state", "chilling"]);
        assert!(r.is_err());
    }

    #[test]
    fn cancel_requires_program_id() {
        let r = Cli::try_parse_from(["vcli", "cancel"]);
        assert!(r.is_err());
    }

    #[test]
    fn cancel_parses_id_and_reason() {
        let c = parse(&[
            "vcli",
            "cancel",
            "12345678-1234-4567-8910-111213141516",
            "--reason",
            "user abort",
        ]);
        match c.command {
            Command::Cancel(a) => {
                assert_eq!(
                    a.program_id.to_string(),
                    "12345678-1234-4567-8910-111213141516"
                );
                assert_eq!(a.reason.as_deref(), Some("user abort"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn cancel_rejects_garbage_id() {
        let r = Cli::try_parse_from(["vcli", "cancel", "not-a-uuid"]);
        assert!(r.is_err());
    }

    #[test]
    fn logs_parses_follow_and_since() {
        let c = parse(&[
            "vcli",
            "logs",
            "12345678-1234-4567-8910-111213141516",
            "--follow",
            "--since",
            "2026-04-16T00:00:00Z",
        ]);
        match c.command {
            Command::Logs(a) => {
                assert!(a.follow);
                assert_eq!(a.since.as_deref(), Some("2026-04-16T00:00:00Z"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn resume_from_start_flag() {
        let c = parse(&[
            "vcli",
            "resume",
            "12345678-1234-4567-8910-111213141516",
            "--from-start",
        ]);
        match c.command {
            Command::Resume(a) => assert!(a.from_start),
            _ => panic!(),
        }
    }

    #[test]
    fn resume_without_flag_defaults_false() {
        let c = parse(&["vcli", "resume", "12345678-1234-4567-8910-111213141516"]);
        match c.command {
            Command::Resume(a) => assert!(!a.from_start),
            _ => panic!(),
        }
    }

    #[test]
    fn daemon_subcommands_all_parse() {
        for arg in ["start", "run", "stop", "status"] {
            let c = parse(&["vcli", "daemon", arg]);
            let ok = matches!(
                (arg, &c.command),
                ("start", Command::Daemon(DaemonCommand::Start))
                    | ("run", Command::Daemon(DaemonCommand::Run))
                    | ("stop", Command::Daemon(DaemonCommand::Stop))
                    | ("status", Command::Daemon(DaemonCommand::Status))
            );
            assert!(ok, "{arg} did not match expected daemon subcommand");
        }
    }

    #[test]
    fn health_and_gc_parse() {
        assert!(matches!(
            parse(&["vcli", "health"]).command,
            Command::Health
        ));
        assert!(matches!(parse(&["vcli", "gc"]).command, Command::Gc));
    }

    #[test]
    fn default_output_mode_is_pretty() {
        let c = parse(&["vcli", "health"]);
        assert_eq!(c.output_mode(), OutputMode::Pretty);
    }
}
