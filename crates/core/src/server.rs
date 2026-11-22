use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::collectors::manager::Collectors;

const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// The HTTP server on 127.0.0.1:<port>, running as a tokio task until `stop`.
pub struct Server {
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
    collectors: Collectors,
}

impl Server {
    pub async fn start(router: Router, collectors: Collectors, port: u16) -> anyhow::Result<Server> {
        let socket = tokio::net::TcpSocket::new_v4()?;
        // Lingering TIME_WAIT connections from the previous run must not block a restart. Not on Windows: there
        // SO_REUSEADDR would let us take a port another program (or another DonateHub) is listening on.
        #[cfg(not(windows))]
        socket.set_reuseaddr(true)?;
        socket.bind(SocketAddr::from(([127, 0, 0, 1], port)))?;
        let listener = socket.listen(1024)?;
        let (shutdown, rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let serve = axum::serve(listener, router).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(e) = serve.await {
                tracing::error!("server stopped: {e}");
            }
        });
        Ok(Server { shutdown, task, collectors })
    }

    /// Stop accepting connections, stop collectors, wait at most 10 s for open requests.
    pub async fn stop(self) {
        let _ = self.shutdown.send(());
        self.collectors.stop().await;
        let abort = self.task.abort_handle();
        if tokio::time::timeout(STOP_TIMEOUT, self.task).await.is_err() {
            tracing::warn!("server did not stop in time, aborting open connections");
            abort.abort();
        }
    }
}
