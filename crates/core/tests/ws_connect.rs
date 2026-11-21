use std::time::{Duration, Instant};

use donatehub_core::collectors::connect_ws;

#[tokio::test]
async fn ws_connect_gives_up_on_a_silent_server() {
    // Accepts TCP but never answers the WebSocket upgrade (like a hung TLS handshake).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    let t = Instant::now();
    let r = tokio::time::timeout(
        Duration::from_secs(5),
        connect_ws(&format!("ws://127.0.0.1:{port}/"), Duration::from_millis(300)),
    )
    .await
    .expect("connect_ws hung");
    assert!(r.is_err());
    assert!(t.elapsed() < Duration::from_secs(2));
}
