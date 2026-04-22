//! Thin wrapper around `vcli_ipc::IpcClient::connect`. The only real job is
//! translating `ENOENT`/`ECONNREFUSED` into a `CliError::DaemonDown` so the
//! `vcli` process exits with code 4 (spec §589).

use std::path::Path;

use vcli_ipc::{IpcClient, IpcError};

use crate::error::{CliError, CliResult};

/// Connect to the daemon at `socket_path`.
///
/// # Errors
/// - `CliError::DaemonDown` if the socket is missing or refused.
/// - `CliError::Generic` for other transport failures.
pub async fn connect(socket_path: &Path) -> CliResult<IpcClient> {
    match IpcClient::connect(socket_path).await {
        Ok(c) => Ok(c),
        Err(IpcError::Io(io_err))
            if matches!(
                io_err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            Err(CliError::DaemonDown(socket_path.display().to_string()))
        }
        Err(e) => Err(CliError::Generic(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ExitCode;

    #[tokio::test]
    async fn connect_to_missing_socket_returns_daemon_down() {
        let p = Path::new("/tmp/vcli-definitely-does-not-exist.sock");
        let e = connect(p).await.unwrap_err();
        assert!(matches!(e, CliError::DaemonDown(_)));
        assert_eq!(e.exit_code(), ExitCode::DaemonDown);
    }
}
