use chrono::{Duration, Utc};
use donatehub_core::currency::RateProvider;
use donatehub_core::store::Store;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const OFFLINE: &str = "http://127.0.0.1:1/daily_json.js";

async fn cbr() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"Valute": {"USD": {"Value": 90.0, "Nominal": 1}, "KZT": {"Value": 18.0, "Nominal": 100}}}),
        ))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn test_rub_passthrough() {
    let server = cbr().await;
    let rp = RateProvider::new(Store::memory(), reqwest::Client::new(), server.uri());
    assert_eq!(rp.to_rub(150.0, "RUB").await, Some(150.0));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_convert_fetches_and_caches() {
    let server = cbr().await;
    let rp = RateProvider::new(Store::memory(), reqwest::Client::new(), server.uri());
    assert_eq!(rp.to_rub(2.0, "USD").await, Some(180.0));
    assert_eq!(rp.to_rub(1000.0, "kzt").await, Some(180.0));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_convert_uses_cached_rate_when_offline() {
    let store = Store::memory();
    let old = (Utc::now() - Duration::days(3)).to_rfc3339();
    store.set_setting("cbr_rates", &json!({"fetched_at": old, "rates": {"USD": 80.0}})).unwrap();
    let rp = RateProvider::new(store, reqwest::Client::new(), OFFLINE.into());
    assert_eq!(rp.to_rub(1.0, "USD").await, Some(80.0));
}

#[tokio::test]
async fn test_unknown_currency_returns_none() {
    let rp = RateProvider::new(Store::memory(), reqwest::Client::new(), OFFLINE.into());
    assert_eq!(rp.to_rub(1.0, "USD").await, None);
}

#[tokio::test]
async fn reads_cache_written_by_python() {
    let store = Store::memory();
    // Python writes datetime.now(UTC).isoformat()
    let fresh = Utc::now().format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string();
    store.set_setting("cbr_rates", &json!({"fetched_at": fresh, "rates": {"USD": 91.5}})).unwrap();
    let rp = RateProvider::new(store, reqwest::Client::new(), OFFLINE.into());
    assert_eq!(rp.to_rub(2.0, "USD").await, Some(183.0));
}
