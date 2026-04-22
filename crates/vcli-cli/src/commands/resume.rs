//! `vcli resume <id> [--from-start]`. Per spec §578, this returns exit 2 if
//! the daemon reports `bad_state_transition` (program not in
//! `failed(daemon_restart)`) or `not_resumable` / `resume_precondition_failed`
//! — all are validation-ish failures, not "not found". That mapping is
//! encoded below: `BadStateTransition` stays `Generic` (exit 1) per
//! `CliError::from_payload`, but we override here because the user-facing
//! resume semantics are closer to a precondition check.

use std::path::Path;

use vcli_core::{ErrorCode, ErrorPayload};
use vcli_ipc::{RequestOp, ResponseBody};

use crate::cli::{OutputMode, ResumeArgs};
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::format::render_value;

/// Run `vcli resume`.
///
/// # Errors
/// See [`CliError`]. `bad_state_transition` / `not_resumable` /
/// `resume_precondition_failed` → exit 2 (validation-class) per spec §578+§589.
pub async fn run(socket: &Path, mode: OutputMode, args: &ResumeArgs) -> CliResult<String> {
    let mut client = connect(socket).await?;
    let resp = client
        .request(RequestOp::Resume {
            program_id: args.program_id,
            from_start: args.from_start,
        })
        .await?;
    match resp.body {
        ResponseBody::Ok { result, .. } => {
            let pretty = summary(&args.program_id.to_string(), &result);
            render_value(mode, &pretty, &result)
        }
        ResponseBody::Err { error, .. } => Err(map_resume_error(&error)),
    }
}

fn summary(id: &str, result: &serde_json::Value) -> String {
    let from_step = result
        .get("from_step")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    format!("resumed {id} from step {from_step}\n")
}

fn map_resume_error(p: &ErrorPayload) -> CliError {
    match p.code {
        ErrorCode::BadStateTransition
        | ErrorCode::NotResumable
        | ErrorCode::ResumePreconditionFailed => CliError::Validation(format_msg(p)),
        _ => CliError::from_payload(p),
    }
}

fn format_msg(p: &ErrorPayload) -> String {
    match &p.path {
        Some(path) => format!("{} (at {})", p.message, path),
        None => p.message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ExitCode;

    #[test]
    fn summary_reports_step() {
        let s = summary("abc", &serde_json::json!({ "from_step": 3 }));
        assert_eq!(s, "resumed abc from step 3\n");
    }

    #[test]
    fn summary_missing_step_defaults_to_zero() {
        let s = summary("abc", &serde_json::json!({}));
        assert!(s.contains("step 0"));
    }

    #[test]
    fn bad_state_transition_becomes_validation_exit_2() {
        let p = ErrorPayload::simple(ErrorCode::BadStateTransition, "program is running");
        let e = map_resume_error(&p);
        assert!(matches!(e, CliError::Validation(_)));
        assert_eq!(e.exit_code(), ExitCode::Validation);
    }

    #[test]
    fn not_resumable_becomes_validation_exit_2() {
        let p = ErrorPayload::simple(ErrorCode::NotResumable, "has unresumable watches");
        assert!(matches!(map_resume_error(&p), CliError::Validation(_)));
    }

    #[test]
    fn resume_precondition_failed_becomes_validation_exit_2() {
        let p = ErrorPayload::simple(
            ErrorCode::ResumePreconditionFailed,
            "step N-1 no longer true",
        );
        assert!(matches!(map_resume_error(&p), CliError::Validation(_)));
    }

    #[test]
    fn unknown_program_still_maps_to_not_found() {
        let p = ErrorPayload::simple(ErrorCode::UnknownProgram, "no such id");
        assert!(matches!(map_resume_error(&p), CliError::NotFound(_)));
    }
}
