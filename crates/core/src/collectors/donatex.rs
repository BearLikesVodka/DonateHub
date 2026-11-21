use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::as_f64;
use super::base::MarkOk;
use super::signalr;
use crate::models::{Donation, Platform, from_iso};
use crate::store::Store;

pub const API: &str = "https://donatex.gg/api";
pub const HUB_URL: &str = "https://donatex.gg/api/public-donations-hub";
pub const PAGE: usize = 100;
pub const MAX_PAGES: usize = 50;

fn timestamp(obj: &Value) -> anyhow::Result<DateTime<Utc>> {
    from_iso(obj["timestamp"].as_str().ok_or_else(|| anyhow!("no timestamp"))?)
}

pub fn parse_dx_donation(obj: &Value) -> anyhow::Result<Donation> {
    let external_id = match &obj["id"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(anyhow!("no id")),
    };
    Ok(Donation {
        platform: Platform::Dx,
        external_id,
        username: obj["username"].as_str().unwrap_or("").to_string(),
        amount: as_f64(&obj["amount"]).unwrap_or(0.0),
        currency: obj["currency"].as_str().filter(|s| !s.is_empty()).unwrap_or("RUB").to_string(),
        amount_rub: as_f64(&obj["amountInRub"]),
        created_at: timestamp(obj)?,
        is_test: obj["isTest"].as_bool().unwrap_or(false),
        raw: obj.clone(),
    })
}

pub struct DonateXCollector {
    store: Store,
    token: String,
    on_new: Arc<dyn Fn() + Send + Sync>,
    client: reqwest::Client,
    api: String,
    hub_url: String,
}

impl DonateXCollector {
    pub fn new(
        store: Store,
        token: String,
        on_new: Arc<dyn Fn() + Send + Sync>,
        client: reqwest::Client,
        api: String,
        hub_url: String,
    ) -> DonateXCollector {
        DonateXCollector { store, token, on_new, client, api, hub_url }
    }

    fn save(&self, obj: &Value) -> bool {
        let d = match parse_dx_donation(obj) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("DonateX: unparseable donation: {e}");
                return false;
            }
        };
        self.store.add_donation(&d).unwrap_or_else(|e| {
            tracing::warn!("DonateX: donation not saved: {e:#}");
            false
        })
    }

    pub fn handle_event(&self, target: &str, args: &[Value]) {
        if matches!(target, "DonationCreated" | "DonationRerun")
            && let Some(obj) = args.first().filter(|a| a.is_object())
            && self.save(obj)
        {
            (self.on_new)();
        }
    }

    pub fn catch_up_since(&self) -> anyhow::Result<Option<DateTime<Utc>>> {
        if let Some(latest) = self.store.latest_donation_time("dx")? {
            return Ok(Some(latest));
        }
        Ok(self.store.active_goal()?.map(|g| g.started_at))
    }

    /// Read REST pages newest-first back to `since`; returns how many donations were new.
    pub async fn catch_up(&self, since: DateTime<Utc>) -> anyhow::Result<usize> {
        let mut added = 0;
        for page in 0..MAX_PAGES {
            let skip = (page * PAGE).to_string();
            let take = PAGE.to_string();
            let items: Vec<Value> = self
                .client
                .get(format!("{}/v1/donations", self.api))
                .query(&[("skip", skip.as_str()), ("take", take.as_str()), ("sortOrder", "NewestFirst")])
                .bearer_auth(&self.token)
                .timeout(Duration::from_secs(15))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let mut reached_boundary = false;
            for obj in &items {
                match timestamp(obj) {
                    Ok(at) if at < since => {
                        reached_boundary = true;
                        break;
                    }
                    Ok(_) => added += self.save(obj) as usize,
                    Err(_) => continue,
                }
            }
            if reached_boundary || items.len() < PAGE {
                break;
            }
        }
        if added > 0 {
            (self.on_new)();
        }
        Ok(added)
    }

    pub async fn connect(&self, mark_ok: MarkOk) -> anyhow::Result<()> {
        signalr::run(
            &self.hub_url,
            &self.token,
            &self.client,
            async || {
                mark_ok();
                // Catch up after the socket is open so a donation can't slip between the REST read and the stream.
                if let Some(since) = self.catch_up_since()? {
                    self.catch_up(since).await?;
                }
                Ok(())
            },
            |target, args| self.handle_event(target, args),
        )
        .await
    }
}
