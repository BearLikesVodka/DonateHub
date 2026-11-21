mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Duration, TimeZone, Utc};
use common::t;
use donatehub_core::collectors::donatex::{DonateXCollector, parse_dx_donation};
use donatehub_core::models::Platform;
use donatehub_core::store::Store;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn doc_example() -> Value {
    json!({
        "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6", "username": "Viewer", "message": "hi",
        "currency": "USD", "amount": 100, "amountInRub": 9200, "timestamp": "2026-10-12T18:42:07Z",
        "wasShown": false, "isTest": false,
    })
}

fn dx(id: &str, at: DateTime<Utc>) -> Value {
    let mut v = doc_example();
    let o = v.as_object_mut().unwrap();
    o.insert("id".into(), json!(id));
    o.insert("amountInRub".into(), json!(100.0));
    o.insert("amount".into(), json!(100.0));
    o.insert("currency".into(), json!("RUB"));
    o.insert("timestamp".into(), json!(at.format("%Y-%m-%dT%H:%M:%SZ").to_string()));
    v
}

fn counter() -> (Arc<AtomicUsize>, Arc<dyn Fn() + Send + Sync>) {
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    (n, Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
    }))
}

fn collector(store: &Store, on_new: Arc<dyn Fn() + Send + Sync>, api: &str) -> DonateXCollector {
    DonateXCollector::new(store.clone(), "tok".into(), on_new, reqwest::Client::new(), api.into(), "http://unused".into())
}

#[test]
fn test_parse_dx_donation() {
    let d = parse_dx_donation(&doc_example()).unwrap();
    assert_eq!(d.platform, Platform::Dx);
    assert_eq!(d.external_id, "3fa85f64-5717-4562-b3fc-2c963f66afa6");
    assert_eq!((d.amount, d.currency.as_str(), d.amount_rub), (100.0, "USD", Some(9200.0)));
    assert_eq!(d.created_at, Utc.with_ymd_and_hms(2026, 10, 12, 18, 42, 7).unwrap());
    assert!(!d.is_test);
    assert_eq!(d.username, "Viewer");
}

#[test]
fn parse_dx_tolerates_numeric_id_and_null_username() {
    let mut v = doc_example();
    v["id"] = json!(42);
    v["username"] = Value::Null;
    v["amountInRub"] = Value::Null;
    let d = parse_dx_donation(&v).unwrap();
    assert_eq!((d.external_id.as_str(), d.username.as_str(), d.amount_rub), ("42", "", None));
    v["timestamp"] = json!("garbage");
    assert!(parse_dx_donation(&v).is_err());
}

#[test]
fn test_rerun_is_deduplicated() {
    let store = Store::memory();
    let (calls, on_new) = counter();
    let c = collector(&store, on_new, "http://unused");
    c.handle_event("DonationCreated", &[dx("a", t(12, 0))]);
    c.handle_event("DonationRerun", &[dx("a", t(12, 0))]);
    c.handle_event("GoalUpdated", &[Value::Null]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.latest_donation_time("dx").unwrap(), Some(t(12, 0)));
}

#[tokio::test]
async fn test_catchup_stops_at_boundary() {
    let server = MockServer::start().await;
    let page: Vec<Value> = (0..100).map(|i| dx(&format!("d{i}"), t(20, 0) - Duration::minutes(10 * i))).collect();
    Mock::given(method("GET"))
        .and(path("/v1/donations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&page))
        .mount(&server)
        .await;
    let store = Store::memory();
    let (calls, on_new) = counter();
    let c = collector(&store, on_new, &server.uri());
    assert_eq!(c.catch_up(t(10, 0)).await.unwrap(), 61);
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.headers["authorization"], "Bearer tok");
    let q: std::collections::HashMap<_, _> = r.url.query_pairs().into_owned().collect();
    assert_eq!((q["sortOrder"].as_str(), q["take"].as_str(), q["skip"].as_str()), ("NewestFirst", "100", "0"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_catchup_pages_until_short_page() {
    let server = MockServer::start().await;
    let first: Vec<Value> = (0..100).map(|i| dx(&format!("a{i}"), t(20, 0) - Duration::seconds(i))).collect();
    Mock::given(query_param("skip", "0")).respond_with(ResponseTemplate::new(200).set_body_json(&first)).mount(&server).await;
    Mock::given(query_param("skip", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([dx("b", t(15, 0))])))
        .mount(&server)
        .await;
    let store = Store::memory();
    let (_, on_new) = counter();
    let c = collector(&store, on_new, &server.uri());
    assert_eq!(c.catch_up(t(10, 0)).await.unwrap(), 101);
    let skips: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.query_pairs().find(|(k, _)| k == "skip").unwrap().1.into_owned())
        .collect();
    assert_eq!(skips, ["0", "100"]);
}

#[test]
fn test_catchup_since_prefers_latest_known() {
    let store = Store::memory();
    let (_, on_new) = counter();
    let c = collector(&store, on_new, "http://unused");
    assert_eq!(c.catch_up_since().unwrap(), None);
    store.create_goal("A", 1000.0, t(8, 0), None).unwrap().unwrap();
    assert_eq!(c.catch_up_since().unwrap(), Some(t(8, 0)));
    c.handle_event("DonationCreated", &[dx("x", t(11, 0))]);
    assert_eq!(c.catch_up_since().unwrap(), Some(t(11, 0)));
}
