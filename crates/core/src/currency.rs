use std::collections::HashMap;
use std::time::Duration;

use chrono::Utc;
use serde_json::{Value, json};

use crate::models::from_iso;
use crate::store::Store;

pub const CBR_URL: &str = "https://www.cbr-xml-daily.ru/daily_json.js";
pub const CACHE_KEY: &str = "cbr_rates";
const MAX_AGE: chrono::Duration = chrono::Duration::hours(24);

/// RUB per unit of a currency, from the Central Bank of Russia daily rates; the last good rates survive outages.
#[derive(Clone)]
pub struct RateProvider {
    store: Store,
    client: reqwest::Client,
    url: String,
}

impl RateProvider {
    pub fn new(store: Store, client: reqwest::Client, url: String) -> RateProvider {
        RateProvider { store, client, url }
    }

    fn cached(&self) -> Option<Value> {
        self.store.get_setting(CACHE_KEY).ok().flatten()
    }

    async fn refresh(&self) {
        match self.fetch().await {
            Ok(rates) => {
                let value = json!({"fetched_at": Utc::now().to_rfc3339(), "rates": rates});
                if let Err(e) = self.store.set_setting(CACHE_KEY, &value) {
                    tracing::warn!("CBR rates not saved: {e}");
                }
            }
            Err(e) => tracing::warn!("CBR rates fetch failed: {e}"),
        }
    }

    async fn fetch(&self) -> anyhow::Result<HashMap<String, f64>> {
        let resp = self.client.get(&self.url).timeout(Duration::from_secs(10)).send().await?.error_for_status()?;
        // The endpoint is served as JS but the body is plain JSON.
        let body: Value = serde_json::from_slice(&resp.bytes().await?)?;
        let valute = body["Valute"].as_object().ok_or_else(|| anyhow::anyhow!("no Valute"))?;
        valute
            .iter()
            .map(|(code, v)| {
                let value = v["Value"].as_f64().ok_or_else(|| anyhow::anyhow!("bad Value for {code}"))?;
                let nominal = v["Nominal"].as_f64().filter(|n| *n != 0.0).ok_or_else(|| anyhow::anyhow!("bad Nominal"))?;
                Ok((code.clone(), value / nominal))
            })
            .collect()
    }

    pub async fn to_rub(&self, amount: f64, currency: &str) -> Option<f64> {
        let currency = currency.to_uppercase();
        if currency == "RUB" {
            return Some(amount);
        }
        let rate = |c: &Option<Value>| c.as_ref().and_then(|c| c["rates"][currency.as_str()].as_f64());
        let mut cached = self.cached();
        let fresh = cached
            .as_ref()
            .and_then(|c| c["fetched_at"].as_str().and_then(|s| from_iso(s).ok()))
            .is_some_and(|at| Utc::now() - at < MAX_AGE);
        if !fresh || rate(&cached).is_none() {
            self.refresh().await;
            cached = self.cached();
        }
        match rate(&cached) {
            Some(r) => Some((amount * r * 100.0).round() / 100.0),
            None => {
                tracing::warn!("no RUB rate for {currency}");
                None
            }
        }
    }
}
