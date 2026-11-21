//! Minimal SignalR client (JSON protocol over WebSocket), enough for the DonateX public hub.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

pub const RS: char = '\u{1e}';
const PING_INTERVAL: Duration = Duration::from_secs(15);
const PING: u64 = 6;

pub fn encode(msg: &Value) -> String {
    format!("{msg}{RS}")
}

pub fn decode(buf: &str) -> Vec<Value> {
    buf.split(RS).filter(|p| !p.trim().is_empty()).filter_map(|p| serde_json::from_str(p).ok()).collect()
}

fn with_query(url: &str, params: &[(&str, &str)]) -> anyhow::Result<Url> {
    let mut u = Url::parse(url)?;
    u.query_pairs_mut().extend_pairs(params);
    Ok(u)
}

async fn negotiate(client: &reqwest::Client, hub_url: &str, token: &str) -> anyhow::Result<Value> {
    let base = hub_url.split('?').next().unwrap_or(hub_url).trim_end_matches('/');
    let url = with_query(&format!("{base}/negotiate"), &[("negotiateVersion", "1"), ("access_token", token)])?;
    let resp = client.post(url).timeout(Duration::from_secs(15)).send().await?.error_for_status()?;
    Ok(resp.json().await?)
}

fn text(msg: Message) -> Option<String> {
    match msg {
        Message::Text(t) => Some(t.to_string()),
        Message::Binary(b) => String::from_utf8(b.to_vec()).ok(),
        _ => None,
    }
}

/// Connect, call `on_open` once the handshake succeeded, then feed every invocation (type 1) to `on_event`.
/// Always ends with an error: the hub closing (type 7), a dropped socket or silence.
pub async fn run(
    hub_url: &str,
    token: &str,
    client: &reqwest::Client,
    on_open: impl AsyncFnOnce() -> anyhow::Result<()>,
    mut on_event: impl FnMut(&str, &[Value]),
) -> anyhow::Result<()> {
    let mut hub_url = hub_url.to_string();
    let mut token = token.to_string();
    let mut neg = negotiate(client, &hub_url, &token).await?;
    if let Some(url) = neg["url"].as_str() {
        // Redirect to a SignalR service.
        hub_url = url.to_string();
        if let Some(t) = neg["accessToken"].as_str() {
            token = t.to_string();
        }
        neg = negotiate(client, &hub_url, &token).await?;
    }
    let conn_id = neg["connectionToken"]
        .as_str()
        .or_else(|| neg["connectionId"].as_str())
        .ok_or_else(|| anyhow!("SignalR negotiate: no connection id"))?
        .to_string();

    let mut ws_url = with_query(&hub_url, &[("id", &conn_id), ("access_token", &token)])?;
    let scheme = match ws_url.scheme() {
        "https" => "wss",
        "http" => "ws",
        s => s,
    }
    .to_string();
    ws_url.set_scheme(&scheme).map_err(|_| anyhow!("bad hub url scheme"))?;
    let (ws, _) = tokio_tungstenite::connect_async(ws_url.as_str()).await?;
    let (sink, mut stream) = ws.split();
    let sink = Arc::new(Mutex::new(sink));

    sink.lock().await.send(Message::Text(encode(&json!({"protocol": "json", "version": 1})).into())).await?;
    let first = tokio::time::timeout(Duration::from_secs(15), stream.next())
        .await
        .context("SignalR handshake timeout")?
        .ok_or_else(|| anyhow!("SignalR closed during handshake"))??;
    let handshake = decode(&text(first).unwrap_or_default());
    if handshake.first().is_none_or(|h| h.get("error").is_some()) {
        bail!("SignalR handshake failed: {handshake:?}");
    }

    let pinger_sink = sink.clone();
    let pinger = tokio::spawn(async move {
        loop {
            tokio::time::sleep(PING_INTERVAL).await;
            if pinger_sink.lock().await.send(Message::Text(encode(&json!({"type": PING})).into())).await.is_err() {
                return;
            }
        }
    });
    let result = async {
        on_open().await?;
        // Messages after the handshake may arrive in the same frame as the handshake response.
        let mut pending: Vec<Value> = handshake.into_iter().skip(1).collect();
        loop {
            for msg in pending {
                match msg["type"].as_u64() {
                    Some(1) => {
                        let args = msg["arguments"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                        on_event(msg["target"].as_str().unwrap_or(""), args);
                    }
                    Some(7) => bail!("SignalR closed: {}", msg["error"].as_str().unwrap_or("no reason")),
                    _ => {}
                }
            }
            let next = tokio::time::timeout(PING_INTERVAL * 4, stream.next())
                .await
                .context("SignalR: no messages from the hub")?
                .ok_or_else(|| anyhow!("SignalR socket closed"))??;
            pending = text(next).map(|t| decode(&t)).unwrap_or_default();
        }
    }
    .await;
    pinger.abort();
    result
}
