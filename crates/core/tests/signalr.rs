use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::extract::{RawQuery, Request};
use axum::routing::{any, post};
use donatehub_core::collectors::signalr::{RS, decode, encode, run};
use serde_json::{Value, json};

#[test]
fn test_encode_appends_record_separator() {
    assert_eq!(encode(&json!({"type": 6})), "{\"type\":6}\u{1e}");
}

#[test]
fn test_decode_multiple_frames() {
    let msgs = decode("{\"type\":6}\u{1e}{\"type\":1,\"target\":\"X\",\"arguments\":[1]}\u{1e}");
    assert_eq!(msgs, vec![json!({"type": 6}), json!({"type": 1, "target": "X", "arguments": [1]})]);
}

#[derive(Default)]
struct Seen {
    negotiate: String,
    ws_query: String,
    handshake: Vec<Value>,
}

#[tokio::test]
async fn test_stream_handshake_and_invocations() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (s1, s2) = (seen.clone(), seen.clone());
    let app = axum::Router::new()
        .route(
            "/hub/negotiate",
            post(move |req: Request| async move {
                s1.lock().unwrap().negotiate = req.uri().to_string();
                axum::Json(json!({"connectionToken": "CT", "negotiateVersion": 1}))
            }),
        )
        .route(
            "/hub",
            any(move |RawQuery(q): RawQuery, ws: WebSocketUpgrade| async move {
                s2.lock().unwrap().ws_query = q.unwrap_or_default();
                let seen = s2.clone();
                ws.on_upgrade(move |mut sock| async move {
                    if let Some(Ok(Message::Text(t))) = sock.recv().await {
                        seen.lock().unwrap().handshake = decode(&t);
                    }
                    let _ = sock.send(Message::Text(format!("{{}}{RS}").into())).await;
                    let two = encode(&json!({"type": 6}))
                        + &encode(&json!({"type": 1, "target": "DonationCreated", "arguments": [{"id": "a"}]}));
                    let _ = sock.send(Message::Text(two.into())).await;
                    let _ = sock.send(Message::Text(encode(&json!({"type": 7, "error": "bye"})).into())).await;
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                })
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let got = Arc::new(Mutex::new(Vec::<Value>::new()));
    let (g1, g2) = (got.clone(), got.clone());
    let err = run(
        &format!("http://127.0.0.1:{port}/hub"),
        "tok",
        &reqwest::Client::new(),
        async move || {
            g1.lock().unwrap().push(json!("open"));
            Ok(())
        },
        move |target: &str, args: &[Value]| g2.lock().unwrap().push(json!([target, args])),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("bye"), "{err:#}");
    assert_eq!(*got.lock().unwrap(), vec![json!("open"), json!(["DonationCreated", [{"id": "a"}]])]);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.handshake, vec![json!({"protocol": "json", "version": 1})]);
    assert!(seen.negotiate.contains("/hub/negotiate?negotiateVersion=1"), "{}", seen.negotiate);
    assert!(seen.negotiate.contains("access_token=tok"));
    assert!(seen.ws_query.contains("id=CT") && seen.ws_query.contains("access_token=tok"), "{}", seen.ws_query);
}

#[tokio::test]
async fn aborting_run_closes_the_socket() {
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<&'static str>();
    let closed_tx = Arc::new(Mutex::new(Some(closed_tx)));
    let app = axum::Router::new()
        .route("/hub/negotiate", post(|| async { axum::Json(json!({"connectionToken": "CT"})) }))
        .route(
            "/hub",
            any(move |ws: WebSocketUpgrade| async move {
                let tx = closed_tx.clone();
                ws.on_upgrade(move |mut sock| async move {
                    let _ = sock.recv().await; // handshake
                    let _ = sock.send(Message::Text(format!("{{}}{RS}").into())).await;
                    // Pings may arrive; anything but a close/EOF keeps waiting.
                    loop {
                        match sock.recv().await {
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                            Some(Ok(_)) => {}
                        }
                    }
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send("closed");
                    }
                })
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (opened_tx, opened_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = run(
            &format!("http://127.0.0.1:{port}/hub"),
            "tok",
            &reqwest::Client::new(),
            async move || {
                let _ = opened_tx.send(());
                Ok(())
            },
            |_: &str, _: &[Value]| {},
        )
        .await;
    });
    opened_rx.await.unwrap();
    task.abort(); // what Collectors::restart does to the old collector
    let got = tokio::time::timeout(std::time::Duration::from_secs(3), closed_rx).await;
    assert_eq!(got.ok().and_then(Result::ok), Some("closed"), "old hub connection stayed open");
}
