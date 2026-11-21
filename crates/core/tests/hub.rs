mod common;

use std::time::Duration;

use common::{make_donation, t};
use donatehub_core::hub::Hub;
use donatehub_core::models::Platform::Dx;
use donatehub_core::store::Store;
use serde_json::json;

#[tokio::test]
async fn snapshot_shape() {
    let store = Store::memory();
    let hub = Hub::new(store.clone());
    let s = hub.snapshot().unwrap();
    assert_eq!(s["goal"], json!(null));
    assert_eq!(s["collected_rub"], json!(0));
    assert!(s["style"]["bar"].is_object());

    store.create_goal("Микрофон", 10000.0, t(10, 0), None).unwrap().unwrap();
    store.add_donation(&make_donation(Dx, "a", Some(100.005), t(11, 0))).unwrap();
    store.add_donation(&make_donation(Dx, "b", Some(0.111), t(11, 0))).unwrap();
    let s = hub.snapshot().unwrap();
    let keys: Vec<_> = s.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["goal", "collected_rub", "style"]);
    let gkeys: Vec<_> = s["goal"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(gkeys, ["id", "title", "target_rub", "started_at", "ends_at", "manual_adjust_rub"]);
    assert_eq!(s["goal"]["started_at"], "2026-11-21T10:00:00.000000+00:00");
    assert_eq!(s["goal"]["ends_at"], json!(null));
    assert_eq!(s["collected_rub"], json!(100.12));
}

#[tokio::test]
async fn test_amount_expires() {
    let store = Store::memory();
    store.create_goal("A", 1000.0, t(10, 0), None).unwrap().unwrap();
    let hub = Hub::new(store);
    hub.set_test_ttl(Duration::from_millis(50));
    let mut rx = hub.subscribe();
    hub.add_test_amount(100.0);
    assert_eq!(hub.snapshot().unwrap()["collected_rub"], json!(100.0));
    let first = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.unwrap().unwrap();
    assert_eq!(first["collected_rub"], json!(100.0));
    let second = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.unwrap().unwrap();
    assert_eq!(second["collected_rub"], json!(0.0));
}

#[tokio::test]
async fn broadcast_drops_dead_clients() {
    let hub = Hub::new(Store::memory());
    let dead = hub.subscribe();
    let mut alive = hub.subscribe();
    drop(dead);
    hub.broadcast();
    let msg = tokio::time::timeout(Duration::from_secs(1), alive.recv()).await.unwrap().unwrap();
    assert_eq!(msg["goal"], json!(null));
}
