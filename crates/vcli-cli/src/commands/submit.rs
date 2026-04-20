//! `vcli submit <file.json> [--watch]`.
//!
//! Flow:
//!   1. Read file (`util::read_program_file`).
//!   2. Local preflight: `vcli_dsl::validate_value`. On failure, exit 2 without
//!      touching the daemon. The daemon re-validates authoritatively; this is
//!      just a UX early-out (Decision F4).
//!   3. Send `RequestOp::Submit`. On `invalid_program` response, map to exit 2.
//!   4. Print the assigned `program_id`.
//!   5. If `--watch`, open a logs stream and drain events until `end_of_stream`
//!      or a terminal `program.completed` / `program.failed` /
//!      `program.state_changed → cancelled`.

use std::io::Write;
use std::path::Path;

use vcli_core::{Event, EventData, ProgramId, ProgramState};
use vcli_dsl::validate_value;
use vcli_ipc::{RequestOp, ResponseBody};

use crate::cli::{OutputMode, SubmitArgs};
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::format::render_value;
use crate::util::{format_unix_ms, read_program_file};

/// Run `vcli submit`. Writes to `out` so tests can capture stdout.
///
/// # Errors
/// See [`CliError`].
pub async fn run<W: Write>(
    socket: &Path,
    mode: OutputMode,
    args: &SubmitArgs,
    out: &mut W,
) -> CliResult<()> {
    let program = read_program_file(&args.file)?;
    if let Err(e) = validate_value(&program) {
        return Err(CliError::Validation(format_dsl_error(&e)));
    }

    let mut client = connect(socket).await?;
    let resp = client.request(RequestOp::Submit { program }).await?;
    let program_id = match resp.body {
        ResponseBody::Ok { result, .. } => {
            let id = extract_program_id(&result)?;
            let line = format!("{id}\n");
            let rendered = render_value(mode, &line, &result)?;
            out.write_all(rendered.as_bytes())?;
            if !rendered.ends_with('\n') {
                out.write_all(b"\n")?;
            }
            id
        }
        ResponseBody::Err { error, .. } => return Err(CliError::from_payload(&error)),
    };

    if args.watch {
        watch_until_terminal(socket, mode, program_id, out).await?;
    }
    Ok(())
}

fn extract_program_id(v: &serde_json::Value) -> CliResult<ProgramId> {
    let s = v
        .get("program_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Generic(format!("daemon response missing program_id: {v}")))?;
    s.parse::<ProgramId>()
        .map_err(|e| CliError::Generic(format!("daemon returned malformed program_id: {e}")))
}

async fn watch_until_terminal<W: Write>(
    socket: &Path,
    mode: OutputMode,
    id: ProgramId,
    out: &mut W,
) -> CliResult<()> {
    let client = connect(socket).await?;
    let mut stream = client
        .request_stream(RequestOp::Logs {
            program_id: id,
            follow: true,
        })
        .await?;
    while let Some(frame) = stream.next_frame().await? {
        if let Some(ev) = frame.event.as_ref() {
            emit(mode, ev, out)?;
            if is_terminal(ev) {
                break;
            }
        }
    }
    Ok(())
}

fn emit<W: Write>(mode: OutputMode, ev: &Event, out: &mut W) -> CliResult<()> {
    match mode {
        OutputMode::Json => {
            serde_json::to_writer(&mut *out, ev)?;
            out.write_all(b"\n")?;
        }
        OutputMode::Pretty => {
            let ts = format_unix_ms(ev.at);
            let raw = serde_json::to_value(&ev.data).unwrap_or(serde_json::Value::Null);
            let kind = raw
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("event");
            writeln!(out, "{ts}  {kind}  {raw}")?;
        }
    }
    Ok(())
}

fn is_terminal(ev: &Event) -> bool {
    match &ev.data {
        EventData::ProgramCompleted { .. } | EventData::ProgramFailed { .. } => true,
        EventData::ProgramStateChanged { to, .. } => matches!(to, ProgramState::Cancelled),
        _ => false,
    }
}

fn format_dsl_error(e: &vcli_dsl::DslError) -> String {
    format!("local preflight: {e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_program_id_happy_path() {
        let v = serde_json::json!({ "program_id": "12345678-1234-4567-8910-111213141516" });
        let id = extract_program_id(&v).unwrap();
        assert_eq!(id.to_string(), "12345678-1234-4567-8910-111213141516");
    }

    #[test]
    fn extract_program_id_missing_field_is_generic_error() {
        let v = serde_json::json!({});
        let e = extract_program_id(&v).unwrap_err();
        assert!(matches!(e, CliError::Generic(_)));
    }

    #[test]
    fn extract_program_id_malformed_is_generic_error() {
        let v = serde_json::json!({ "program_id": "not-a-uuid" });
        let e = extract_program_id(&v).unwrap_err();
        assert!(matches!(e, CliError::Generic(_)));
    }

    #[test]
    fn completed_event_is_terminal() {
        let ev = Event {
            at: 0_i64,
            data: EventData::ProgramCompleted {
                program_id: ProgramId::new(),
                emit: None,
            },
        };
        assert!(is_terminal(&ev));
    }

    #[test]
    fn failed_event_is_terminal() {
        let ev = Event {
            at: 0_i64,
            data: EventData::ProgramFailed {
                program_id: ProgramId::new(),
                reason: "boom".into(),
                step: None,
                emit: None,
            },
        };
        assert!(is_terminal(&ev));
    }

    #[test]
    fn state_changed_to_cancelled_is_terminal() {
        let ev = Event {
            at: 0_i64,
            data: EventData::ProgramStateChanged {
                program_id: ProgramId::new(),
                from: ProgramState::Running,
                to: ProgramState::Cancelled,
                reason: "user".into(),
            },
        };
        assert!(is_terminal(&ev));
    }

    #[test]
    fn state_changed_to_running_is_not_terminal() {
        let ev = Event {
            at: 0_i64,
            data: EventData::ProgramStateChanged {
                program_id: ProgramId::new(),
                from: ProgramState::Waiting,
                to: ProgramState::Running,
                reason: "trigger".into(),
            },
        };
        assert!(!is_terminal(&ev));
    }

    #[test]
    fn watch_fired_is_not_terminal() {
        let ev = Event {
            at: 0_i64,
            data: EventData::WatchFired {
                program_id: ProgramId::new(),
                watch_index: 0,
                predicate: "inline".into(),
            },
        };
        assert!(!is_terminal(&ev));
    }
}
