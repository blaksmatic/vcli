//! vcli-cli — user-facing command-line interface for the vcli daemon.
//!
//! See `docs/superpowers/specs/2026-04-16-vcli-design.md` §567–590 for the
//! authoritative CLI surface. This crate is a thin transport+formatting layer
//! over `vcli-ipc`; the daemon does all real work.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod cli;
pub mod client;
pub mod commands;
pub mod error;
pub mod format;
pub mod util;

pub use client::connect;
pub use format::{render_value, Row, Table};

pub use cli::{Cli, Command, DaemonCommand, OutputMode, StateFilter};
pub use error::{CliError, CliResult, ExitCode};
pub use util::{format_unix_ms, read_program_file, resolve_socket};

/// Entry point used by `src/bin/vcli.rs`. Returns the process exit code.
#[must_use]
pub fn run() -> i32 {
    let cli = match <Cli as clap::Parser>::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let code = if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                ExitCode::Success
            } else {
                ExitCode::Generic
            };
            let _ = e.print();
            return code.into();
        }
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("vcli: tokio runtime init failed: {e}");
            return ExitCode::Generic.into();
        }
    };

    let result = runtime.block_on(run_async(cli));
    match result {
        Ok(()) => ExitCode::Success.into(),
        Err(e) => {
            eprintln!("vcli: {e}");
            e.exit_code().into()
        }
    }
}

async fn run_async(cli: Cli) -> CliResult<()> {
    use std::io::Write as _;

    let socket = crate::util::resolve_socket(cli.socket.as_deref())?;
    let mode = cli.output_mode();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    match &cli.command {
        Command::Submit(args) => crate::commands::submit::run(&socket, mode, args, &mut out).await,
        Command::List(args) => {
            let s = crate::commands::list::run(&socket, mode, args).await?;
            out.write_all(s.as_bytes())?;
            Ok(())
        }
        Command::Cancel(args) => {
            let s = crate::commands::cancel::run(&socket, mode, args).await?;
            out.write_all(s.as_bytes())?;
            Ok(())
        }
        Command::Logs(args) => crate::commands::logs::run(&socket, mode, args, &mut out).await,
        Command::Resume(args) => {
            let s = crate::commands::resume::run(&socket, mode, args).await?;
            out.write_all(s.as_bytes())?;
            Ok(())
        }
        Command::Daemon(sub) => crate::commands::daemon::run(&socket, mode, sub, &mut out).await,
        Command::Health => {
            let s = crate::commands::health::run(&socket, mode).await?;
            out.write_all(s.as_bytes())?;
            Ok(())
        }
        Command::Gc => {
            let s = crate::commands::gc::run(&socket, mode).await?;
            out.write_all(s.as_bytes())?;
            Ok(())
        }
    }
}
