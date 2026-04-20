//! `vcli daemon { start | run | stop | status }`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use tokio::time::{sleep, Instant};
use vcli_ipc::{RequestOp, ResponseBody};

use crate::cli::{DaemonCommand, OutputMode};
use crate::client::connect;
use crate::error::{CliError, CliResult};
use crate::format::render_value;

const DAEMON_BINARY: &str = "vcli-daemon";

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
        DaemonCommand::Start => start(socket, mode, out).await,
        DaemonCommand::Run => run_foreground(),
    }
}

async fn start<W: Write>(socket: &Path, mode: OutputMode, out: &mut W) -> CliResult<()> {
    if socket.exists() {
        let json = serde_json::json!({ "started": false, "reason": "already_running" });
        let rendered = render_value(mode, "daemon already running\n", &json)?;
        out.write_all(rendered.as_bytes())?;
        return Ok(());
    }
    spawn_detached(DAEMON_BINARY).map_err(|e| CliError::Generic(format!("spawn daemon: {e}")))?;
    wait_for_socket(socket, Duration::from_secs(5)).await?;
    let json = serde_json::json!({ "started": true, "socket": socket.display().to_string() });
    let rendered = render_value(mode, "daemon started\n", &json)?;
    out.write_all(rendered.as_bytes())?;
    Ok(())
}

fn run_foreground() -> CliResult<()> {
    use std::os::unix::process::CommandExt as _;
    let err = Command::new(DAEMON_BINARY).exec();
    Err(CliError::Generic(format!("exec {DAEMON_BINARY}: {err}")))
}

fn spawn_detached(bin: &str) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt as _;
    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    cmd.spawn()?;
    Ok(())
}

async fn wait_for_socket(path: &Path, timeout: Duration) -> CliResult<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    Err(CliError::Generic(format!(
        "timed out waiting for {} to appear",
        path.display()
    )))
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

    #[tokio::test]
    async fn start_is_idempotent_when_socket_already_exists() {
        let dir = tempdir().unwrap();
        let sock = dir.path().join("vcli.sock");
        std::fs::write(&sock, "").unwrap();
        let mut buf: Vec<u8> = Vec::new();
        start(&sock, OutputMode::Pretty, &mut buf).await.unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("already running"), "{out}");
    }

    #[tokio::test]
    async fn start_reports_timeout_when_daemon_binary_absent() {
        // Why: spawn_detached will fail when `vcli-daemon` is not on PATH,
        // so start() bubbles up CliError::Generic rather than waiting the full
        // 5s. Relies on PATH not containing a real daemon during `cargo test`.
        let dir = tempdir().unwrap();
        let sock = dir.path().join("vcli.sock");
        let mut buf: Vec<u8> = Vec::new();
        let err = start(&sock, OutputMode::Pretty, &mut buf)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Generic(_)));
    }

    #[tokio::test]
    async fn wait_for_socket_returns_ok_when_file_appears() {
        let dir = tempdir().unwrap();
        let sock = dir.path().join("vcli.sock");
        let sock2 = sock.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            std::fs::write(&sock2, "").unwrap();
        });
        wait_for_socket(&sock, Duration::from_millis(500))
            .await
            .unwrap();
        task.await.unwrap();
    }
}
