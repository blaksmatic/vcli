//! `vcli cancel <id> [--reason TEXT]`. Exit 3 on unknown id (mapped through
//! `CliError::from_payload`).

use std::path::Path;

use vcli_ipc::{RequestOp, ResponseBody};

use crate::cli::{CancelArgs, OutputMode};
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::format::render_value;

/// Run `vcli cancel`.
///
/// # Errors
/// See [`CliError`]. `unknown_program` → exit 3 via `CliError::NotFound`.
pub async fn run(socket: &Path, mode: OutputMode, args: &CancelArgs) -> CliResult<String> {
    let mut client = connect(socket).await?;
    let resp = client
        .request(RequestOp::Cancel {
            program_id: args.program_id,
        })
        .await?;
    match resp.body {
        ResponseBody::Ok { result, .. } => {
            let pretty = summary(&args.program_id.to_string(), args.reason.as_deref(), &result);
            render_value(mode, &pretty, &result)
        }
        ResponseBody::Err { error, .. } => Err(CliError::from_payload(&error)),
    }
}

fn summary(id: &str, reason: Option<&str>, result: &serde_json::Value) -> String {
    let state = result
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("cancelled");
    match reason {
        Some(r) => format!("cancelled {id} → {state} (reason: {r})\n"),
        None => format!("cancelled {id} → {state}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::{ErrorCode, ErrorPayload};

    #[test]
    fn summary_without_reason() {
        let s = summary("aaa", None, &serde_json::json!({ "state": "cancelled" }));
        assert_eq!(s, "cancelled aaa → cancelled\n");
    }

    #[test]
    fn summary_with_reason() {
        let s = summary(
            "aaa",
            Some("user abort"),
            &serde_json::json!({ "state": "cancelled" }),
        );
        assert!(s.contains("user abort"));
    }

    #[test]
    fn unknown_program_payload_maps_to_not_found() {
        let p = ErrorPayload::simple(ErrorCode::UnknownProgram, "no such program");
        let e = CliError::from_payload(&p);
        assert!(matches!(e, CliError::NotFound(_)));
        assert_eq!(e.exit_code(), crate::error::ExitCode::NotFound);
    }
}
