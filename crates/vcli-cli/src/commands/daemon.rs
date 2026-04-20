//! `vcli daemon { start | run | stop | status }`. Start/run in Task 15.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::time::{sleep, Instant};
use vcli_ipc::{RequestOp, ResponseBody};

use crate::cli::{DaemonCommand, OutputMode};
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::format::render_value;

/// Top-level dispatcher for `vcli daemon …`.
///
/// # Errors
/// See [`CliError`].
pub async fn run<W: Write>(
    socket: &Path,
    mode: OutputMode,
    cmd: &DaemonCommand,
    out: &mut W,
) -> CliResult<()> {
    match cmd {
        DaemonCommand::Status => status(socket, mode, out).await,
        DaemonCommand::Stop => stop(socket, mode, out).await,
        DaemonCommand::Start | DaemonCommand::Run => Err(CliError::Generic(
            "daemon start/run not yet implemented".into(),
        )),
    }
}

async fn status<W: Write>(socket: &Path, mode: OutputMode, out: &mut W) -> CliResult<()> {
    let exists = socket.exists();
    let (running, detail) = if exists {
        match connect(socket).await {
            Ok(mut c) => match c.request(RequestOp::Health).await {
                Ok(resp) => match resp.body {
                    ResponseBody::Ok { result, .. } => (true, result),
                    ResponseBody::Err { error, .. } => {
                        (true, serde_json::json!({ "error": error.message }))
                    }
                },
                Err(e) => (false, serde_json::json!({ "error": e.to_string() })),
            },
            Err(CliError::DaemonDown(_)) => (false, serde_json::Value::Null),
            Err(e) => return Err(e),
        }
    } else {
        (false, serde_json::Value::Null)
    };
    let json = serde_json::json!({
        "running": running,
        "socket": socket.display().to_string(),
        "detail": detail,
    });
    let pretty = if running {
        format!("running\nsocket: {}\n", socket.display())
    } else {
        format!("not running\nsocket: {}\n", socket.display())
    };
    let rendered = render_value(mode, &pretty, &json)?;
    out.write_all(rendered.as_bytes())?;
    if !rendered.ends_with('\n') {
        out.write_all(b"\n")?;
    }
    Ok(())
}

async fn stop<W: Write>(socket: &Path, mode: OutputMode, out: &mut W) -> CliResult<()> {
    if !socket.exists() {
        let json = serde_json::json!({ "stopped": false, "reason": "not_running" });
        let rendered = render_value(mode, "daemon not running\n", &json)?;
        out.write_all(rendered.as_bytes())?;
        return Ok(());
    }
    let mut client = connect(socket).await?;
    let _ = client.request(RequestOp::Shutdown).await;
    wait_for_socket_gone(socket, Duration::from_secs(5)).await?;
    let json = serde_json::json!({ "stopped": true });
    let rendered = render_value(mode, "stopped\n", &json)?;
    out.write_all(rendered.as_bytes())?;
    Ok(())
}

async fn wait_for_socket_gone(path: &Path, timeout: Duration) -> CliResult<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !path.exists() {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    Err(CliError::Generic(format!(
        "timed out waiting for {} to disappear",
        path.display()
    )))
}

// Why: will be called by Task 15 `start` to locate an existing pidfile.
#[allow(dead_code)]
pub(crate) fn pid_file_candidate(socket: &Path) -> Option<PathBuf> {
    socket.parent().map(|p| p.join("vcli.pid"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn status_reports_not_running_when_socket_missing() {
        let dir = tempdir().unwrap();
        let sock = dir.path().join("vcli.sock");
        let mut buf: Vec<u8> = Vec::new();
        status(&sock, OutputMode::Pretty, &mut buf).await.unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("not running"), "{out}");
    }

    #[tokio::test]
    async fn status_json_mode_emits_running_field() {
        let dir = tempdir().unwrap();
        let sock = dir.path().join("vcli.sock");
        let mut buf: Vec<u8> = Vec::new();
        status(&sock, OutputMode::Json, &mut buf).await.unwrap();
        let out = String::from_utf8(buf).unwrap();
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["running"], false);
    }

    #[tokio::test]
    async fn stop_with_no_socket_is_not_running() {
        let dir = tempdir().unwrap();
        let sock = dir.path().join("vcli.sock");
        let mut buf: Vec<u8> = Vec::new();
        stop(&sock, OutputMode::Pretty, &mut buf).await.unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("not running"), "{out}");
    }

    #[tokio::test]
    async fn wait_for_socket_gone_times_out_when_file_stays() {
        let dir = tempdir().unwrap();
        let sock = dir.path().join("vcli.sock");
        std::fs::write(&sock, "").unwrap();
        let err = wait_for_socket_gone(&sock, Duration::from_millis(120))
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Generic(_)));
    }

    #[test]
    fn pid_file_candidate_is_alongside_socket() {
        let p = pid_file_candidate(Path::new("/tmp/foo/vcli.sock")).unwrap();
        assert_eq!(p, PathBuf::from("/tmp/foo/vcli.pid"));
    }
}
