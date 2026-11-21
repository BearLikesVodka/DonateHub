pub mod base;
pub mod donationalerts;
pub mod donatex;
pub mod manager;
pub mod signalr;

/// JSON number, or a string holding one (platform APIs are not consistent about it).
pub(crate) fn as_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Aborts the task when dropped, so helper tasks die with the collector future that owns them
/// (an aborted collector drops its future without running any code after the current `.await`).
pub(crate) struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// How long a platform WebSocket may take to connect (TCP + TLS + upgrade) before the collector retries.
pub const WS_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub async fn connect_ws(url: &str, timeout: std::time::Duration) -> anyhow::Result<WsStream> {
    let (ws, _) = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(url))
        .await
        .map_err(|_| anyhow::anyhow!("WebSocket connect timeout"))??;
    Ok(ws)
}
