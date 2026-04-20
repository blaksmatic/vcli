//! Grab-bag of helpers — socket resolution honouring `--socket`, ISO-8601
//! formatting of `UnixMs`, and program-file ingestion with byte-for-byte
//! limits so we don't OOM on an accidental multi-GB file.

use std::fs;
use std::path::{Path, PathBuf};

use vcli_core::clock::UnixMs;
use vcli_ipc::default_socket_path;

use crate::error::{CliError, CliResult};

/// Hard cap on submitted program size. Matches `vcli_ipc::MAX_FRAME_LEN` minus
/// envelope overhead headroom.
pub const MAX_PROGRAM_BYTES: u64 = 2 * 1024 * 1024;

/// Resolve the socket path. Explicit override (`--socket` / `VCLI_SOCKET`)
/// wins; otherwise falls through `vcli_ipc::default_socket_path`.
///
/// # Errors
/// Returns `CliError::Generic` if no path is discoverable.
pub fn resolve_socket(explicit: Option<&Path>) -> CliResult<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    default_socket_path()
        .map(|s| s.path)
        .map_err(|e| CliError::Generic(format!("resolve socket: {e}")))
}

/// Read a program JSON file from disk. Enforces `MAX_PROGRAM_BYTES` before
/// allocating — an accidental `vcli submit /dev/urandom` should fail cheaply.
///
/// # Errors
/// Returns `CliError::Generic` if the file is missing or oversize, and
/// `CliError::Validation` if the bytes aren't valid UTF-8 JSON.
pub fn read_program_file(path: &Path) -> CliResult<serde_json::Value> {
    let meta = fs::metadata(path)
        .map_err(|e| CliError::Generic(format!("stat {}: {e}", path.display())))?;
    if meta.len() > MAX_PROGRAM_BYTES {
        return Err(CliError::Generic(format!(
            "program file too large: {} bytes (max {MAX_PROGRAM_BYTES})",
            meta.len()
        )));
    }
    let bytes =
        fs::read(path).map_err(|e| CliError::Generic(format!("read {}: {e}", path.display())))?;
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|e| CliError::Validation(format!("invalid JSON in {}: {e}", path.display())))
}

/// Format a `UnixMs` as an RFC 3339 / ISO 8601 UTC timestamp.
///
/// Computed directly rather than pulling in `chrono`/`time`; the only use
/// site is display.
#[must_use]
pub fn format_unix_ms(ms: UnixMs) -> String {
    let total_secs = ms.div_euclid(1_000);
    let sub_ms = ms.rem_euclid(1_000);

    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;

    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{sub_ms:03}Z")
}

/// Howard Hinnant's days-from-civil inverse: convert days since 1970-01-01
/// to (year, month, day). Public-domain.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u8, d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolve_socket_honours_explicit_override() {
        let p = Path::new("/tmp/explicit.sock");
        let got = resolve_socket(Some(p)).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/explicit.sock"));
    }

    #[test]
    fn read_program_file_roundtrips_small_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("p.json");
        fs::write(&path, r#"{"name":"x"}"#).unwrap();
        let v = read_program_file(&path).unwrap();
        assert_eq!(v["name"], "x");
    }

    #[test]
    fn read_program_file_reports_missing_as_generic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let e = read_program_file(&path).unwrap_err();
        assert!(matches!(e, CliError::Generic(_)));
    }

    #[test]
    fn read_program_file_reports_bad_json_as_validation() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("p.json");
        fs::write(&path, "{not json").unwrap();
        let e = read_program_file(&path).unwrap_err();
        assert!(matches!(e, CliError::Validation(_)));
    }

    #[test]
    fn format_unix_ms_epoch_is_1970_01_01() {
        let s = format_unix_ms(0_i64);
        assert_eq!(s, "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn format_unix_ms_known_instant() {
        // 2026-04-16 19:42:11.123 UTC.
        let ms = 20_559_i64 * 86_400_000 + (19 * 3600 + 42 * 60 + 11) * 1000 + 123;
        let s = format_unix_ms(ms);
        assert_eq!(s, "2026-04-16T19:42:11.123Z");
    }
}
