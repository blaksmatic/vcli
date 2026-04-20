//! Shared fake-daemon harness for e2e tests.

use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::oneshot;

use vcli_ipc::handler::test_double::FakeHandler;
use vcli_ipc::IpcServer;

pub struct FakeDaemon {
    pub socket: PathBuf,
    // Why: kept so tests can inspect which ops the handler received. Not all
    // suites need it yet, but removing it would force future tests to re-add.
    #[allow(dead_code)]
    pub handler: Arc<FakeHandler>,
    pub shutdown: Option<oneshot::Sender<()>>,
    // Why: TempDir is load-bearing RAII — dropping it here would unlink the
    // socket mid-test. The field itself is never read.
    #[allow(dead_code)]
    pub tmp: TempDir,
}

impl FakeDaemon {
    pub async fn start() -> Self {
        let tmp = TempDir::new().unwrap();
        let socket = tmp.path().join("vcli.sock");
        let handler = Arc::new(FakeHandler::default());
        let server = IpcServer::bind(&socket, handler.clone()).unwrap();
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = server.serve(rx).await;
        });
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        Self {
            socket,
            handler,
            shutdown: Some(tx),
            tmp,
        }
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}
