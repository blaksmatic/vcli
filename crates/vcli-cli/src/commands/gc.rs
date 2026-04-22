//! `vcli gc` — one-shot `RequestOp::Gc` → "ok" or JSON.

use std::path::Path;

use vcli_ipc::{RequestOp, ResponseBody};

use crate::cli::OutputMode;
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::format::render_value;

/// Run `vcli gc`.
///
/// # Errors
/// See [`CliError`].
pub async fn run(socket: &Path, mode: OutputMode) -> CliResult<String> {
    let mut client = connect(socket).await?;
    let resp = client.request(RequestOp::Gc).await?;
    match resp.body {
        ResponseBody::Ok { result, .. } => {
            let pretty = summary(&result);
            render_value(mode, &pretty, &result)
        }
        ResponseBody::Err { error, .. } => Err(CliError::from_payload(&error)),
    }
}

fn summary(v: &serde_json::Value) -> String {
    let removed = v
        .get("removed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let bytes = v
        .get("bytes_freed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    format!("gc: ok (removed {removed}, freed {bytes} bytes)\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_with_counts() {
        let s = summary(&serde_json::json!({ "removed": 5, "bytes_freed": 1024 }));
        assert!(s.contains("removed 5"));
        assert!(s.contains("1024"));
    }

    #[test]
    fn summary_with_no_fields_is_still_ok() {
        let s = summary(&serde_json::json!({}));
        assert!(s.starts_with("gc: ok"));
    }
}
