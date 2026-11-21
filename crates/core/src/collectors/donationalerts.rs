use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, NaiveDateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::as_f64;
use super::base::MarkOk;
use crate::currency::RateProvider;
use crate::models::{Donation, Platform};
use crate::store::Store;

pub const SITE: &str = "https://www.donationalerts.com";
pub const CENTRIFUGO_URL: &str = "wss://centrifugo.donationalerts.com/connection/websocket";
pub const SCOPES: &str = "oauth-user-show oauth-donation-subscribe oauth-donation-index";
pub const SETTINGS_KEY: &str = "da";
const REFRESH_MARGIN: f64 = 86400.0;
const MAX_PAGES: u32 = 50;
const CENTRIFUGO_PING: Duration = Duration::from_secs(25);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

fn now() -> f64 {
    Utc::now().timestamp_millis() as f64 / 1000.0
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/auth/da/callback")
}

pub fn authorize_url(client_id: &str, state: &str, redirect: &str) -> String {
    let q = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("client_id", client_id),
            ("redirect_uri", redirect),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("state", state),
        ])
        .finish();
    format!("{SITE}/oauth/authorize?{q}")
}

fn cfg_str<'a>(cfg: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    cfg[key].as_str().ok_or_else(|| anyhow!("DonationAlerts: в настройках нет {key}"))
}

fn apply_token(cfg: &Value, resp: &Value) -> anyhow::Result<Value> {
    let mut out: Map<String, Value> = cfg.as_object().cloned().unwrap_or_default();
    let access = resp["access_token"].as_str().ok_or_else(|| anyhow!("DonationAlerts: нет access_token в ответе"))?;
    out.insert("access_token".into(), json!(access));
    // DA may omit a new refresh token; the old one stays valid then.
    let refresh = resp["refresh_token"].as_str().filter(|s| !s.is_empty()).map(|s| json!(s));
    out.insert("refresh_token".into(), refresh.unwrap_or_else(|| cfg["refresh_token"].clone()));
    let expires_in = as_f64(&resp["expires_in"]).filter(|x| *x > 0.0).unwrap_or(3600.0);
    out.insert("expires_at".into(), json!(now() + expires_in));
    Ok(Value::Object(out))
}

async fn token_request(client: &reqwest::Client, site: &str, form: &[(&str, &str)]) -> anyhow::Result<Value> {
    let resp = client.post(format!("{site}/oauth/token")).form(form).timeout(HTTP_TIMEOUT).send().await?;
    Ok(resp.error_for_status()?.json().await?)
}

pub async fn exchange_code(
    client: &reqwest::Client,
    site: &str,
    cfg: &Value,
    code: &str,
    redirect: &str,
) -> anyhow::Result<Value> {
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", cfg_str(cfg, "client_id")?),
        ("client_secret", cfg_str(cfg, "client_secret")?),
        ("redirect_uri", redirect),
        ("code", code),
    ];
    apply_token(cfg, &token_request(client, site, &form).await?)
}

pub async fn refresh(client: &reqwest::Client, site: &str, cfg: &Value) -> anyhow::Result<Value> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", cfg_str(cfg, "refresh_token")?),
        ("client_id", cfg_str(cfg, "client_id")?),
        ("client_secret", cfg_str(cfg, "client_secret")?),
        ("scope", SCOPES),
    ];
    apply_token(cfg, &token_request(client, site, &form).await?)
}

/// Docs say "YYYY-MM-DD HH.MM.SS", examples use colons; DA times are UTC.
pub fn parse_created_at(s: &str) -> anyhow::Result<DateTime<Utc>> {
    let (date, clock) = s.trim().split_once(' ').ok_or_else(|| anyhow!("bad created_at {s:?}"))?;
    let text = format!("{date}T{}", clock.replace('.', ":"));
    let naive = NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S%.f"))?;
    Ok(naive.and_utc())
}

fn currency(obj: &Value) -> String {
    obj["currency"].as_str().filter(|s| !s.is_empty()).unwrap_or("RUB").to_uppercase()
}

pub fn parse_da_donation(obj: &Value, amount_rub: Option<f64>) -> anyhow::Result<Donation> {
    let external_id = match &obj["id"] {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => bail!("no id"),
    };
    Ok(Donation {
        platform: Platform::Da,
        external_id,
        username: obj["username"].as_str().unwrap_or("").to_string(),
        amount: as_f64(&obj["amount"]).ok_or_else(|| anyhow!("no amount"))?,
        currency: currency(obj),
        amount_rub,
        created_at: parse_created_at(obj["created_at"].as_str().ok_or_else(|| anyhow!("no created_at"))?)?,
        is_test: false,
        raw: obj.clone(),
    })
}

fn is_donation(x: &Value) -> bool {
    x.get("id").is_some() && x.get("amount").is_some()
}

pub fn extract_push_donation(msg: &Value) -> Option<Value> {
    let data = msg.get("result")?.get("data")?.as_object()?;
    if let Some(inner) = data.get("data").filter(|d| is_donation(d)) {
        return Some(inner.clone());
    }
    let data = Value::Object(data.clone());
    is_donation(&data).then_some(data)
}

/// Centrifugo frames may hold several newline-separated JSON messages.
fn messages(frame: &str) -> Vec<Value> {
    frame
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| match serde_json::from_str(l) {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!("DonationAlerts: bad frame");
                None
            }
        })
        .collect()
}

fn frame_text(msg: Message) -> Option<String> {
    match msg {
        Message::Text(t) => Some(t.to_string()),
        Message::Binary(b) => String::from_utf8(b.to_vec()).ok(),
        _ => None,
    }
}

pub struct DACollector {
    store: Store,
    rates: RateProvider,
    on_new: Arc<dyn Fn() + Send + Sync>,
    client: reqwest::Client,
    site: String,
    centrifugo_url: String,
    ping_interval: Duration,
}

impl DACollector {
    pub fn new(
        store: Store,
        rates: RateProvider,
        on_new: Arc<dyn Fn() + Send + Sync>,
        client: reqwest::Client,
        site: String,
        centrifugo_url: String,
    ) -> DACollector {
        DACollector { store, rates, on_new, client, site, centrifugo_url, ping_interval: CENTRIFUGO_PING }
    }

    /// Centrifugo ping period; silence for three periods means a dead connection.
    pub fn with_ping_interval(mut self, interval: Duration) -> DACollector {
        self.ping_interval = interval;
        self
    }

    pub async fn ensure_token(&self) -> anyhow::Result<String> {
        let mut cfg = self.store.get_setting(SETTINGS_KEY)?.unwrap_or_else(|| json!({}));
        if cfg["access_token"].as_str().is_none_or(str::is_empty) {
            bail!("DonationAlerts: нужно войти через OAuth");
        }
        let expires_at = as_f64(&cfg["expires_at"]).unwrap_or(0.0);
        let has_refresh = cfg["refresh_token"].as_str().is_some_and(|s| !s.is_empty());
        if expires_at - now() < REFRESH_MARGIN && has_refresh {
            cfg = refresh(&self.client, &self.site, &cfg).await?;
            self.store.set_setting(SETTINGS_KEY, &cfg)?;
        }
        Ok(cfg_str(&cfg, "access_token")?.to_string())
    }

    async fn api(&self, req: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let token = self.ensure_token().await?;
        let resp = req.bearer_auth(token).timeout(HTTP_TIMEOUT).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.site)
    }

    async fn save(&self, obj: &Value) -> bool {
        let Some(amount) = as_f64(&obj["amount"]) else {
            tracing::warn!("DonationAlerts: unparseable donation (no amount)");
            return false;
        };
        let rub = self.rates.to_rub(amount, &currency(obj)).await;
        let d = match parse_da_donation(obj, rub) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("DonationAlerts: unparseable donation: {e}");
                return false;
            }
        };
        if d.amount_rub.is_none() {
            tracing::warn!("DonationAlerts: no RUB rate for {}, donation {} saved without amount", d.currency, d.external_id);
        }
        self.store.add_donation(&d).unwrap_or_else(|e| {
            tracing::warn!("DonationAlerts: donation not saved: {e:#}");
            false
        })
    }

    pub async fn handle_message(&self, msg: &Value) -> anyhow::Result<()> {
        let Some(obj) = extract_push_donation(msg) else {
            let res = &msg["result"];
            let is_sub_ack = res["data"].get("info").is_some();
            if res["channel"].as_str().is_some_and(|c| !c.is_empty()) && !is_sub_ack {
                tracing::warn!("DonationAlerts: unrecognized push {msg}");
            }
            return Ok(());
        };
        if self.save(&obj).await {
            (self.on_new)();
        }
        Ok(())
    }

    pub fn catch_up_since(&self) -> anyhow::Result<Option<DateTime<Utc>>> {
        if let Some(latest) = self.store.latest_donation_time("da")? {
            return Ok(Some(latest));
        }
        Ok(self.store.active_goal()?.map(|g| g.started_at))
    }

    pub async fn catch_up(&self, since: DateTime<Utc>) -> anyhow::Result<usize> {
        let mut added = 0;
        for page in 1..=MAX_PAGES {
            let page = page.to_string();
            let body = self.api(self.client.get(self.url("/alerts/donations")).query(&[("page", &page)])).await?;
            let mut reached = false;
            for obj in body["data"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                match obj["created_at"].as_str().map(parse_created_at) {
                    Some(Ok(at)) if at < since => {
                        reached = true;
                        break;
                    }
                    Some(Ok(_)) => added += self.save(obj).await as usize,
                    _ => continue,
                }
            }
            let has_next = body["links"]["next"].as_str().is_some_and(|s| !s.is_empty());
            if reached || !has_next {
                break;
            }
        }
        if added > 0 {
            (self.on_new)();
        }
        Ok(added)
    }

    pub async fn fix_missing_rub(&self) -> anyhow::Result<usize> {
        let mut fixed = 0;
        for d in self.store.donations_missing_rub()? {
            if let Some(rub) = self.rates.to_rub(d.amount, &d.currency).await {
                self.store.set_amount_rub(d.platform.as_str(), &d.external_id, rub)?;
                fixed += 1;
            }
        }
        if fixed > 0 {
            (self.on_new)();
        }
        Ok(fixed)
    }

    pub async fn connect(&self, mark_ok: MarkOk) -> anyhow::Result<()> {
        let user = self.api(self.client.get(self.url("/user/oauth"))).await?;
        let user = &user["data"];
        let user_id = match &user["id"] {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => bail!("DonationAlerts: нет id пользователя"),
        };
        let socket_token = user["socket_connection_token"].as_str().ok_or_else(|| anyhow!("no socket token"))?;
        let channel = format!("$alerts:donation_{user_id}");

        let ws = super::connect_ws(&self.centrifugo_url, super::WS_CONNECT_TIMEOUT).await?;
        let (mut sink, mut stream) = ws.split();
        let send = |v: Value| Message::Text(v.to_string().into());
        sink.send(send(json!({"params": {"token": socket_token}, "id": 1}))).await?;
        let client_id = loop {
            let frame = tokio::time::timeout(HTTP_TIMEOUT, stream.next())
                .await
                .context("Centrifugo connect timeout")?
                .ok_or_else(|| anyhow!("Centrifugo closed"))??;
            let found = messages(&frame_text(frame).unwrap_or_default()).into_iter().find(|m| m["id"] == 1);
            if let Some(m) = found {
                if !m["error"].is_null() {
                    bail!("Centrifugo connect: {}", m["error"]);
                }
                break m["result"]["client"].as_str().ok_or_else(|| anyhow!("Centrifugo: no client id"))?.to_string();
            }
        };
        let sub = self
            .api(self.client.post(self.url("/centrifuge/subscribe")).json(&json!({"channels": [channel], "client": client_id})))
            .await?;
        let chan_token = sub["channels"]
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["channel"] == channel.as_str()))
            .and_then(|c| c["token"].as_str())
            .ok_or_else(|| anyhow!("Centrifugo: no channel token"))?;
        sink.send(send(json!({"params": {"channel": channel, "token": chan_token}, "method": 1, "id": 2}))).await?;

        mark_ok();
        if let Some(since) = self.catch_up_since()? {
            self.catch_up(since).await?;
        }
        self.fix_missing_rub().await?;

        let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + self.ping_interval, self.ping_interval);
        let hour = Duration::from_secs(3600);
        let mut fix_rub = tokio::time::interval_at(tokio::time::Instant::now() + hour, hour);
        let mut n = 100;
        // Our pings get answers, so silence for 3 ping periods means a dead connection.
        // One deadline for the whole loop: it moves only when a frame arrives, not on every ping tick.
        let silence = tokio::time::sleep(self.ping_interval * 3);
        tokio::pin!(silence);
        loop {
            tokio::select! {
                _ = ping.tick() => {
                    n += 1;
                    sink.send(send(json!({"method": 7, "id": n}))).await?;
                }
                _ = fix_rub.tick() => {
                    if let Err(e) = self.fix_missing_rub().await {
                        tracing::warn!("DonationAlerts: fix rub failed: {e:#}");
                    }
                }
                _ = &mut silence => bail!("Centrifugo: no messages"),
                frame = stream.next() => {
                    let frame = frame.ok_or_else(|| anyhow!("Centrifugo closed"))??;
                    silence.as_mut().reset(tokio::time::Instant::now() + self.ping_interval * 3);
                    for msg in messages(&frame_text(frame).unwrap_or_default()) {
                        if msg["id"] == 2 && !msg["error"].is_null() {
                            bail!("Centrifugo subscribe: {}", msg["error"]);
                        }
                        self.handle_message(&msg).await?;
                    }
                }
            }
        }
    }
}
