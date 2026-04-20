//! `vcli logs <id> [--follow] [--since ISO8601]` — streams `Event`s scoped to
//! a single program until `end_of_stream` or the user hits Ctrl-C.

use std::io::Write;
use std::path::Path;

use vcli_core::Event;
use vcli_ipc::{RequestOp, StreamFrame};

use crate::cli::{LogsArgs, OutputMode};
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::util::format_unix_ms;

/// Run `vcli logs`. Writes to `out` instead of `stdout` so tests can capture.
///
/// # Errors
/// See [`CliError`].
pub async fn run<W: Write>(
    socket: &Path,
    mode: OutputMode,
    args: &LogsArgs,
    out: &mut W,
) -> CliResult<()> {
    let client = connect(socket).await?;
    let mut stream = client
        .request_stream(RequestOp::Logs {
            program_id: args.program_id,
            follow: args.follow,
        })
        .await?;
    let since_ms = parse_since(args.since.as_deref())?;
    while let Some(frame) = stream.next_frame().await? {
        if let Some(ev) = frame.event.as_ref() {
            if since_ms.is_some_and(|t| ev.at < t) {
                continue;
            }
            write_event(mode, &frame, ev, out)?;
        }
    }
    Ok(())
}

fn write_event<W: Write>(
    mode: OutputMode,
    frame: &StreamFrame,
    ev: &Event,
    out: &mut W,
) -> CliResult<()> {
    match mode {
        OutputMode::Json => {
            serde_json::to_writer(&mut *out, frame)?;
            out.write_all(b"\n")?;
        }
        OutputMode::Pretty => {
            let line = format_event_line(ev);
            out.write_all(line.as_bytes())?;
        }
    }
    Ok(())
}

fn format_event_line(ev: &Event) -> String {
    let ts = format_unix_ms(ev.at);
    let raw = serde_json::to_value(&ev.data).unwrap_or(serde_json::Value::Null);
    let kind = raw
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("event");
    format!("{ts}  {kind}  {raw}\n")
}

fn parse_since(s: Option<&str>) -> CliResult<Option<i64>> {
    let Some(s) = s else {
        return Ok(None);
    };
    parse_rfc3339_utc(s)
        .map(Some)
        .ok_or_else(|| CliError::Generic(format!("--since: unrecognized timestamp: {s}")))
}

fn parse_rfc3339_utc(s: &str) -> Option<i64> {
    if !s.ends_with('Z') {
        return None;
    }
    let core = &s[..s.len() - 1];
    let (date, time) = core.split_once('T')?;
    let mut dparts = date.split('-');
    let year: i32 = dparts.next()?.parse().ok()?;
    let month: u32 = dparts.next()?.parse().ok()?;
    let day: u32 = dparts.next()?.parse().ok()?;
    let (time, frac) = match time.split_once('.') {
        Some((t, f)) => (t, f),
        None => (time, "0"),
    };
    let mut tparts = time.split(':');
    let hour: u32 = tparts.next()?.parse().ok()?;
    let minute: u32 = tparts.next()?.parse().ok()?;
    let sec: u32 = tparts.next()?.parse().ok()?;
    let millis: u32 = frac.parse().ok()?;
    let days = days_from_civil(year, month, day)?;
    Some(
        days * 86_400_000
            + i64::from(hour) * 3_600_000
            + i64::from(minute) * 60_000
            + i64::from(sec) * 1_000
            + i64::from(millis),
    )
}

#[allow(clippy::cast_possible_wrap)]
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = u32::try_from(year - era * 400).ok()?;
    let doy = (153 * if month > 2 { month - 3 } else { month + 9 } + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era) * 146_097 + i64::from(doe) - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::{Event, EventData};

    #[test]
    fn format_event_line_has_timestamp_and_kind() {
        let ev = Event {
            at: 0_i64,
            data: EventData::DaemonStarted {
                version: "0.0.1".into(),
            },
        };
        let s = format_event_line(&ev);
        assert!(s.contains("1970-01-01T00:00:00.000Z"));
        assert!(s.contains("daemon.started"));
    }

    #[test]
    fn parse_since_accepts_our_own_iso_format() {
        let t = parse_since(Some("1970-01-01T00:00:00.000Z"))
            .unwrap()
            .unwrap();
        assert_eq!(t, 0);
    }

    #[test]
    fn parse_since_without_millis_also_works() {
        let t = parse_since(Some("1970-01-01T00:00:00Z")).unwrap().unwrap();
        assert_eq!(t, 0);
    }

    #[test]
    fn parse_since_rejects_missing_z() {
        let e = parse_since(Some("1970-01-01T00:00:00")).unwrap_err();
        assert!(matches!(e, CliError::Generic(_)));
    }

    #[test]
    fn parse_since_rejects_garbage() {
        let e = parse_since(Some("not a timestamp")).unwrap_err();
        assert!(matches!(e, CliError::Generic(_)));
    }

    #[test]
    fn parse_since_none_is_ok_none() {
        assert!(parse_since(None).unwrap().is_none());
    }
}
