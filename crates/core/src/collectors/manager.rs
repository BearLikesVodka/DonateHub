use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::base::{MarkOk, SharedStatus, describe_error, real_sleep, run_forever};
use super::donationalerts::{self as da, DACollector};
use super::donatex::{self, DonateXCollector};
use crate::currency::{CBR_URL, RateProvider};
use crate::hub::Hub;
use crate::store::Store;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the manager needs from a platform collector.
pub trait Collector: Send + Sync {
    fn catch_up(&self, since: DateTime<Utc>) -> BoxFut<'_, anyhow::Result<usize>>;
    fn connect(&self, mark_ok: MarkOk) -> BoxFut<'_, anyhow::Result<()>>;
}

impl Collector for DonateXCollector {
    fn catch_up(&self, since: DateTime<Utc>) -> BoxFut<'_, anyhow::Result<usize>> {
        Box::pin(DonateXCollector::catch_up(self, since))
    }
    fn connect(&self, mark_ok: MarkOk) -> BoxFut<'_, anyhow::Result<()>> {
        Box::pin(DonateXCollector::connect(self, mark_ok))
    }
}

impl Collector for DACollector {
    fn catch_up(&self, since: DateTime<Utc>) -> BoxFut<'_, anyhow::Result<usize>> {
        Box::pin(DACollector::catch_up(self, since))
    }
    fn connect(&self, mark_ok: MarkOk) -> BoxFut<'_, anyhow::Result<()>> {
        Box::pin(DACollector::connect(self, mark_ok))
    }
}

/// External service addresses; tests point them at local mocks.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub dx_api: String,
    pub dx_hub: String,
    pub da_site: String,
    pub centrifugo: String,
    pub cbr: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            dx_api: donatex::API.into(),
            dx_hub: donatex::HUB_URL.into(),
            da_site: da::SITE.into(),
            centrifugo: da::CENTRIFUGO_URL.into(),
            cbr: CBR_URL.into(),
        }
    }
}

pub const NAMES: [&str; 2] = ["da", "dx"];

#[derive(Clone)]
pub struct Collectors {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    hub: Hub,
    port: u16,
    endpoints: Endpoints,
    client: reqwest::Client,
    rates: RateProvider,
    status: HashMap<&'static str, SharedStatus>,
    state: Mutex<Running>,
}

#[derive(Default)]
struct Running {
    enabled: bool,
    tasks: HashMap<&'static str, JoinHandle<()>>,
    instances: HashMap<&'static str, Arc<dyn Collector>>,
}

fn static_name(name: &str) -> &'static str {
    NAMES.into_iter().find(|n| *n == name).expect("unknown collector")
}

impl Collectors {
    pub fn new(store: Store, hub: Hub, port: u16, endpoints: Endpoints) -> Collectors {
        let client = reqwest::Client::new();
        let rates = RateProvider::new(store.clone(), client.clone(), endpoints.cbr.clone());
        let status = NAMES.into_iter().map(|n| (n, SharedStatus::default())).collect();
        Collectors {
            inner: Arc::new(Inner { store, hub, port, endpoints, client, rates, status, state: Mutex::default() }),
        }
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.inner.endpoints
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.inner.client
    }

    pub fn start(&self) {
        self.inner.state.lock().unwrap().enabled = true;
        for n in NAMES {
            self.restart(n);
        }
    }

    pub async fn stop(&self) {
        let tasks: Vec<_> = {
            let mut st = self.inner.state.lock().unwrap();
            st.enabled = false;
            st.instances.clear();
            st.tasks.drain().map(|(_, t)| t).collect()
        };
        for t in &tasks {
            t.abort();
        }
        for t in tasks {
            let _ = t.await;
        }
    }

    fn configured(&self, name: &str) -> bool {
        let cfg = self.inner.store.get_setting(name).ok().flatten().unwrap_or(Value::Null);
        let key = if name == "dx" { "token" } else { "access_token" };
        cfg[key].as_str().is_some_and(|s| !s.is_empty())
    }

    pub fn restart(&self, name: &str) {
        let name = static_name(name);
        let mut st = self.inner.state.lock().unwrap();
        if let Some(old) = st.tasks.remove(name) {
            old.abort();
        }
        st.instances.remove(name);
        let status = self.inner.status[name].clone();
        if !st.enabled || !self.configured(name) {
            *status.lock().unwrap() = Default::default();
            return;
        }
        let i = &self.inner;
        let collector: Arc<dyn Collector> = if name == "dx" {
            let token = i.store.get_setting("dx").ok().flatten().unwrap_or(Value::Null)["token"]
                .as_str()
                .unwrap_or("")
                .to_string();
            Arc::new(DonateXCollector::new(
                i.store.clone(),
                token,
                i.hub.notifier(),
                i.client.clone(),
                i.endpoints.dx_api.clone(),
                i.endpoints.dx_hub.clone(),
            ))
        } else {
            Arc::new(DACollector::new(
                i.store.clone(),
                i.rates.clone(),
                i.hub.notifier(),
                i.client.clone(),
                i.endpoints.da_site.clone(),
                i.endpoints.centrifugo.clone(),
            ))
        };
        st.instances.insert(name, collector.clone());
        let task = tokio::spawn(async move {
            run_forever(
                name,
                move |mark_ok| {
                    let c = collector.clone();
                    async move { c.connect(mark_ok).await }
                },
                status,
                real_sleep,
            )
            .await
        });
        st.tasks.insert(name, task);
    }

    /// Replace a running collector instance (tests use it to observe backfills).
    pub fn set_instance(&self, name: &str, collector: Arc<dyn Collector>) {
        self.inner.state.lock().unwrap().instances.insert(static_name(name), collector);
    }

    /// Fetch donations back to `since` (e.g. a goal start moved into the past) without blocking the request.
    pub fn backfill(&self, since: DateTime<Utc>) {
        let instances: Vec<_> =
            self.inner.state.lock().unwrap().instances.iter().map(|(n, c)| (*n, c.clone())).collect();
        for (name, c) in instances {
            tokio::spawn(async move {
                if let Err(e) = c.catch_up(since).await {
                    tracing::warn!("{name} backfill failed: {}", describe_error(&e));
                }
            });
        }
    }

    pub fn platforms(&self) -> Value {
        let da_cfg = self.inner.store.get_setting("da").ok().flatten().unwrap_or(Value::Null);
        let entry = |name: &str| {
            let s = self.inner.status[name].lock().unwrap().clone();
            json!({"configured": self.configured(name), "state": s.state, "error": s.error})
        };
        let mut da_entry = entry("da");
        let client_id = da_cfg["client_id"].as_str().unwrap_or("");
        let has_client = !client_id.is_empty() && da_cfg["client_secret"].as_str().is_some_and(|s| !s.is_empty());
        da_entry["has_client"] = json!(has_client);
        da_entry["client_id"] = json!(client_id);
        da_entry["redirect_uri"] = json!(da::redirect_uri(self.inner.port));
        json!({"da": da_entry, "dx": entry("dx")})
    }
}
