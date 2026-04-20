//! Output rendering. Two modes: `Pretty` (tables / single-line summaries) and
//! `Json` (one `serde_json::Value` serialized once). Commands pick by reading
//! `Cli::output_mode()`; the helpers in this file do no I/O — they return
//! `String`s the caller writes to `stdout`.

use std::fmt::Write as _;

use crate::cli::OutputMode;
use crate::error::CliError;

/// Simple table — header + rows. All string data; commands format numbers
/// and timestamps before inserting.
#[derive(Debug, Clone)]
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Row>,
}

/// One tabular row.
#[derive(Debug, Clone)]
pub struct Row(pub Vec<String>);

impl Table {
    /// Build a table. `headers.len()` is the enforced column count.
    #[must_use]
    pub fn new<I, S>(headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            headers: headers.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
        }
    }

    /// Append one row. Extra cells are truncated; missing cells are filled
    /// with empty strings so the output is always rectangular.
    pub fn push(&mut self, row: Row) {
        let mut cells = row.0;
        cells.resize(self.headers.len(), String::new());
        self.rows.push(Row(cells));
    }

    /// Render to a string. Columns are left-aligned, padded to the widest cell.
    #[must_use]
    pub fn render_pretty(&self) -> String {
        let cols = self.headers.len();
        let mut widths = vec![0usize; cols];
        for (i, h) in self.headers.iter().enumerate() {
            widths[i] = h.chars().count();
        }
        for row in &self.rows {
            for (i, cell) in row.0.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        let mut out = String::new();
        for (i, h) in self.headers.iter().enumerate() {
            let _ = write!(out, "{:<w$}", h, w = widths[i]);
            if i + 1 < cols {
                out.push_str("  ");
            }
        }
        out.push('\n');
        for row in &self.rows {
            for (i, cell) in row.0.iter().enumerate() {
                let _ = write!(out, "{:<w$}", cell, w = widths[i]);
                if i + 1 < cols {
                    out.push_str("  ");
                }
            }
            out.push('\n');
        }
        out
    }
}

/// Render a `serde_json::Value` per output mode. `pretty` is the caller's
/// choice of how to humanize the value (for `list` it's a `Table`, for
/// `health` it's a multi-line summary).
///
/// # Errors
/// Returns `CliError::Generic` only if JSON re-serialization fails.
pub fn render_value(
    mode: OutputMode,
    pretty: &str,
    json: &serde_json::Value,
) -> Result<String, CliError> {
    match mode {
        OutputMode::Pretty => Ok(pretty.to_string()),
        OutputMode::Json => Ok(serde_json::to_string_pretty(json)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table_renders_just_header() {
        let t = Table::new(["id", "kind", "state"]);
        let s = t.render_pretty();
        assert_eq!(s.lines().count(), 1);
        assert!(s.contains("id"));
        assert!(s.contains("state"));
    }

    #[test]
    fn table_columns_align_to_widest_cell() {
        let mut t = Table::new(["id", "state"]);
        t.push(Row(vec!["abcdef".into(), "running".into()]));
        t.push(Row(vec!["xy".into(), "completed".into()]));
        let s = t.render_pretty();
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(lines.len(), 3);
        let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        assert_eq!(widths[0], widths[1]);
        assert_eq!(widths[1], widths[2]);
    }

    #[test]
    fn short_rows_get_padded() {
        let mut t = Table::new(["a", "b", "c"]);
        t.push(Row(vec!["1".into()]));
        let s = t.render_pretty();
        assert!(s.lines().count() == 2);
    }

    #[test]
    fn render_value_pretty_returns_pretty_arg() {
        let v = serde_json::json!({ "x": 1 });
        let s = render_value(OutputMode::Pretty, "human readable", &v).unwrap();
        assert_eq!(s, "human readable");
    }

    #[test]
    fn render_value_json_serializes_value() {
        let v = serde_json::json!({ "x": 1 });
        let s = render_value(OutputMode::Json, "ignored", &v).unwrap();
        assert!(s.contains(r#""x": 1"#), "{s}");
    }
}
