//! `vcli list [--state STATE]` — tabular list of programs.

use std::path::Path;

use vcli_ipc::{RequestOp, ResponseBody};

use crate::cli::{ListArgs, OutputMode};
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::format::{render_value, Row, Table};

/// Run `vcli list`.
///
/// # Errors
/// See [`CliError`].
pub async fn run(socket: &Path, mode: OutputMode, args: &ListArgs) -> CliResult<String> {
    let mut client = connect(socket).await?;
    let op = RequestOp::List {
        state: args.state.map(|s| s.as_wire().to_string()),
    };
    let resp = client.request(op).await?;
    match resp.body {
        ResponseBody::Ok { result, .. } => render_value(mode, &format_table(&result), &result),
        ResponseBody::Err { error, .. } => Err(CliError::from_payload(&error)),
    }
}

fn format_table(v: &serde_json::Value) -> String {
    let items = v.get("items").and_then(serde_json::Value::as_array);
    let mut t = Table::new(["id", "name", "state", "submitted_at"]);
    if let Some(items) = items {
        for item in items {
            t.push(Row(vec![
                item.get("program_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                item.get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                item.get("state")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                item.get("submitted_at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ]));
        }
    }
    t.render_pretty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_shows_header_only() {
        let s = format_table(&serde_json::json!({ "items": [] }));
        assert_eq!(s.lines().count(), 1);
        assert!(s.contains("id"));
        assert!(s.contains("state"));
    }

    #[test]
    fn list_with_two_items_prints_both_rows() {
        let v = serde_json::json!({
            "items": [
                { "program_id": "aaa", "name": "yt", "state": "running",
                  "submitted_at": "2026-04-16T19:42:11.123Z" },
                { "program_id": "bbb", "name": "buy", "state": "completed",
                  "submitted_at": "2026-04-16T19:42:12.000Z" }
            ]
        });
        let s = format_table(&v);
        assert_eq!(s.lines().count(), 3);
        assert!(s.contains("aaa"));
        assert!(s.contains("buy"));
        assert!(s.contains("running"));
        assert!(s.contains("completed"));
    }

    #[test]
    fn missing_fields_do_not_panic() {
        let v = serde_json::json!({ "items": [{ "program_id": "x" }] });
        let s = format_table(&v);
        assert!(s.contains('x'));
    }
}
