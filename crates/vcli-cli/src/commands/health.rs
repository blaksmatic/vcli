//! `vcli health` — one-shot `RequestOp::Health` → pretty or JSON summary.

use std::path::Path;

use vcli_ipc::{RequestOp, ResponseBody};

use crate::cli::OutputMode;
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::format::render_value;

/// Run `vcli health`.
///
/// # Errors
/// See [`CliError`].
pub async fn run(socket: &Path, mode: OutputMode) -> CliResult<String> {
    let mut client = connect(socket).await?;
    let resp = client.request(RequestOp::Health).await?;
    match resp.body {
        ResponseBody::Ok { result, .. } => render_value(mode, &pretty(&result), &result),
        ResponseBody::Err { error, .. } => Err(CliError::from_payload(&error)),
    }
}

fn pretty(v: &serde_json::Value) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    out.push_str("daemon: ok\n");
    if let Some(version) = v.get("version").and_then(|x| x.as_str()) {
        let _ = writeln!(out, "version: {version}");
    }
    if let Some(uptime) = v.get("uptime_ms").and_then(serde_json::Value::as_u64) {
        let _ = writeln!(out, "uptime_ms: {uptime}");
    }
    if let Some(progs) = v.get("programs") {
        let _ = writeln!(out, "programs: {progs}");
    }
    if let Some(last_err) = v.get("last_error") {
        if !last_err.is_null() {
            let _ = writeln!(out, "last_error: {last_err}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_includes_known_fields() {
        let v = serde_json::json!({
            "version": "0.0.1",
            "uptime_ms": 12_345_u64,
            "programs": { "running": 2, "waiting": 0 },
            "last_error": null
        });
        let s = pretty(&v);
        assert!(s.contains("daemon: ok"));
        assert!(s.contains("version: 0.0.1"));
        assert!(s.contains("uptime_ms: 12345"));
        assert!(s.contains("running"));
        assert!(!s.contains("last_error:"));
    }

    #[test]
    fn pretty_shows_last_error_when_present() {
        let v = serde_json::json!({ "last_error": "capture_failed" });
        let s = pretty(&v);
        assert!(s.contains("last_error"));
    }
}
