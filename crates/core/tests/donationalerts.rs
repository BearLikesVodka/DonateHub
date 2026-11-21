mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Duration, TimeZone, Utc};
use common::{make_donation, t};
use donatehub_core::collectors::donationalerts::{
    DACollector, authorize_url, extract_push_donation, parse_created_at, parse_da_donation, redirect_uri,
};
use donatehub_core::currency::RateProvider;
use donatehub_core::models::Platform;
use donatehub_core::store::Store;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OFFLINE: &str = "http://127.0.0.1:1/";

fn now() -> f64 {
    Utc::now().timestamp_millis() as f64 / 1000.0
}

fn da(id: i64, at: DateTime<Utc>, amount: f64, currency: &str) -> Value {
    json!({"id": id, "name": "donation", "username": "Viewer", "message_type": "text", "message": "hi",
           "amount": amount, "currency": currency, "is_shown": 1,
           "created_at": at.format("%Y-%m-%d %H:%M:%S").to_string(), "shown_at": null})
}

fn configure(store: &Store, expires_at: f64) {
    store
        .set_setting("da", &json!({"client_id": "7", "client_secret": "sec", "access_token": "AT",
                                   "refresh_token": "RT", "expires_at": expires_at}))
        .unwrap();
}

/// Real RateProvider that knows only USD = 90 and cannot reach the CBR.
fn rates(store: &Store, usd: bool) -> RateProvider {
    let rates = if usd { json!({"USD": 90.0}) } else { json!({}) };
    store.set_setting("cbr_rates", &json!({"fetched_at": Utc::now().to_rfc3339(), "rates": rates})).unwrap();
    RateProvider::new(store.clone(), reqwest::Client::new(), OFFLINE.into())
}

fn counter() -> (Arc<AtomicUsize>, Arc<dyn Fn() + Send + Sync>) {
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    (n, Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
    }))
}

fn collector(store: &Store, usd: bool, on_new: Arc<dyn Fn() + Send + Sync>, site: &str) -> DACollector {
    DACollector::new(store.clone(), rates(store, usd), on_new, reqwest::Client::new(), site.into(), "ws://unused".into())
}

#[test]
fn test_parse_da_created_at_formats() {
    let expected = Utc.with_ymd_and_hms(2026, 11, 21, 9, 5, 7).unwrap();
    assert_eq!(parse_created_at("2026-11-21 09.05.07").unwrap(), expected);
    assert_eq!(parse_created_at("2026-11-21 09:05:07").unwrap(), expected);
}

#[test]
fn test_parse_da_donation() {
    let d = parse_da_donation(&da(5, t(12, 0), 2.0, "usd"), Some(180.0)).unwrap();
    assert_eq!((d.platform, d.external_id.as_str()), (Platform::Da, "5"));
    assert_eq!((d.amount, d.currency.as_str(), d.amount_rub), (2.0, "USD", Some(180.0)));
    assert_eq!(d.created_at, t(12, 0));
    assert!(!d.is_test);
}

#[test]
fn test_extract_push_donation() {
    let push = json!({"result": {"channel": "$alerts:donation_3", "data": {"seq": 1, "data": da(5, t(12, 0), 100.0, "RUB")}}});
    assert_eq!(extract_push_donation(&push).unwrap()["id"], 5);
    let flat = json!({"result": {"channel": "$alerts:donation_3", "data": da(6, t(12, 0), 100.0, "RUB")}});
    assert_eq!(extract_push_donation(&flat).unwrap()["id"], 6);
    let sub_ack = json!({"result": {"type": 1, "channel": "$alerts:donation_3", "data": {"info": {"user": "1"}}}});
    assert!(extract_push_donation(&sub_ack).is_none());
    assert!(extract_push_donation(&json!({"id": 1, "result": {"client": "x"}})).is_none());
}

#[test]
fn test_authorize_url() {
    assert_eq!(redirect_uri(47321), "http://127.0.0.1:47321/auth/da/callback");
    let u = url::Url::parse(&authorize_url("7", "st", &redirect_uri(47321))).unwrap();
    assert_eq!((u.host_str(), u.path()), (Some("www.donationalerts.com"), "/oauth/authorize"));
    let q: HashMap<_, _> = u.query_pairs().into_owned().collect();
    assert_eq!((q["client_id"].as_str(), q["response_type"].as_str(), q["state"].as_str()), ("7", "code", "st"));
    assert_eq!(q["redirect_uri"], "http://127.0.0.1:47321/auth/da/callback");
    assert_eq!(q["scope"], "oauth-user-show oauth-donation-subscribe oauth-donation-index");
}

#[tokio::test]
async fn test_refresh_keeps_old_refresh_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token_type": "Bearer", "access_token": "NEW", "expires_in": 3600})))
        .mount(&server)
        .await;
    let store = Store::memory();
    configure(&store, now() + 60.0);
    let (_, on_new) = counter();
    let c = collector(&store, true, on_new, &server.uri());
    assert_eq!(c.ensure_token().await.unwrap(), "NEW");
    let cfg = store.get_setting("da").unwrap().unwrap();
    assert_eq!((cfg["access_token"].as_str(), cfg["refresh_token"].as_str()), (Some("NEW"), Some("RT")));
    assert!(cfg["expires_at"].as_f64().unwrap() > now() + 3000.0);
    assert_eq!(cfg["client_secret"], "sec");
    let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
    let form: HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes()).into_owned().collect();
    assert_eq!((form["grant_type"].as_str(), form["refresh_token"].as_str()), ("refresh_token", "RT"));
}

#[tokio::test]
async fn test_ensure_token_no_refresh_when_fresh() {
    let store = Store::memory();
    configure(&store, now() + 365.0 * 86400.0);
    let (_, on_new) = counter();
    let c = collector(&store, true, on_new, OFFLINE);
    assert_eq!(c.ensure_token().await.unwrap(), "AT");
}

#[tokio::test]
async fn test_catchup_stops_at_boundary() {
    let server = MockServer::start().await;
    let mut page1: Vec<Value> = (0..30).map(|i| da(100 - i, t(20, 0) - Duration::minutes(30 * i), 100.0, "RUB")).collect();
    page1[1] = da(99, t(19, 30), 2.0, "USD");
    Mock::given(path("/api/v1/alerts/donations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": page1, "links": {"next": "https://x/?page=2"}, "meta": {}})))
        .mount(&server)
        .await;
    let store = Store::memory();
    configure(&store, now() + 365.0 * 86400.0);
    let (calls, on_new) = counter();
    let c = collector(&store, true, on_new, &server.uri());
    assert_eq!(c.catch_up(t(10, 0)).await.unwrap(), 21); // t(20) back to t(10) inclusive, every 30 minutes
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].headers["authorization"], "Bearer AT");
    assert_eq!(reqs[0].url.query_pairs().find(|(k, _)| k == "page").unwrap().1, "1");
    let g = store.create_goal("A", 1.0, t(19, 0), None).unwrap().unwrap();
    assert_eq!(store.collected(&g).unwrap(), 100.0 + 180.0 + 100.0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_catchup_follows_next_link() {
    let server = MockServer::start().await;
    Mock::given(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [da(2, t(15, 0), 100.0, "RUB")], "links": {"next": "https://x/?page=2"}})))
        .mount(&server)
        .await;
    Mock::given(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [da(1, t(14, 0), 100.0, "RUB")], "links": {"next": null}})))
        .mount(&server)
        .await;
    let store = Store::memory();
    configure(&store, now() + 365.0 * 86400.0);
    let (_, on_new) = counter();
    let c = collector(&store, true, on_new, &server.uri());
    assert_eq!(c.catch_up(t(10, 0)).await.unwrap(), 2);
}

#[tokio::test]
async fn test_fix_missing_rub() {
    let store = Store::memory();
    let mut d = make_donation(Platform::Da, "u1", None, t(12, 0));
    d.amount = 2.0;
    d.currency = "USD".into();
    store.add_donation(&d).unwrap();
    let (calls, on_new) = counter();
    let c = collector(&store, true, on_new, OFFLINE);
    assert_eq!(c.fix_missing_rub().await.unwrap(), 1);
    assert!(store.donations_missing_rub().unwrap().is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_push_saves_donation_and_converts() {
    let store = Store::memory();
    let (calls, on_new) = counter();
    let c = collector(&store, false, on_new, OFFLINE);
    let msg = json!({"result": {"channel": "$alerts:donation_3", "data": {"data": da(7, t(12, 0), 5.0, "EUR")}}});
    c.handle_message(&msg).await.unwrap();
    c.handle_message(&msg).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let missing = store.donations_missing_rub().unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].external_id, "7");
}
