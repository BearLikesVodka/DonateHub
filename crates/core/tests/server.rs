use std::time::{Duration, Instant};

use donatehub_core::app::build;
use donatehub_core::config::port_free;
use donatehub_core::server::Server;
use donatehub_core::store::Store;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[tokio::test]
async fn test_server_thread_stop_releases_port() {
    let port = free_port();
    let (router, state) = build(Store::memory(), false, port);
    let srv = Server::start(router, state.collectors.clone(), port).await.unwrap();
    let r = reqwest::get(format!("http://127.0.0.1:{port}/api/state")).await.unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["port"]["current"], port);
    assert!(!port_free(port));
    // An overlay stays connected while the app quits.
    let (_ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
    let t = Instant::now();
    srv.stop().await;
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    assert!(port_free(port));
}

#[tokio::test]
async fn start_fails_on_busy_port() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (router, state) = build(Store::memory(), false, port);
    assert!(Server::start(router, state.collectors, port).await.is_err());
}

#[tokio::test]
async fn second_server_on_same_port_fails() {
    // Guards Windows, where SO_REUSEADDR would let a second listener take a busy port.
    let port = free_port();
    let (router, state) = build(Store::memory(), false, port);
    let first = Server::start(router, state.collectors.clone(), port).await.unwrap();
    let (router2, state2) = build(Store::memory(), false, port);
    assert!(Server::start(router2, state2.collectors, port).await.is_err());
    first.stop().await;
}

#[tokio::test]
async fn server_restarts_on_same_port_right_after_stop() {
    // An update or quick quit+relaunch restarts while the old server's closed connections sit in TIME_WAIT.
    let port = free_port();
    let (router, state) = build(Store::memory(), false, port);
    let srv = Server::start(router, state.collectors.clone(), port).await.unwrap();
    let r = reqwest::Client::new().get(format!("http://127.0.0.1:{port}/api/state")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let (_ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
    srv.stop().await;
    assert!(port_free(port), "the app's own port check must not report a busy port");
    let (router, state) = build(Store::memory(), false, port);
    let again = Server::start(router, state.collectors.clone(), port).await.unwrap();
    again.stop().await;
}
