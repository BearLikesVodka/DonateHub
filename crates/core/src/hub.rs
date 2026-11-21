use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::models::to_iso;
use crate::store::Store;
use crate::style::{default_style, merge_style};

const DEFAULT_TEST_TTL: Duration = Duration::from_secs(15);

/// Current widget state and fan-out of it to every connected overlay.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    tx: broadcast::Sender<Value>,
    test_ttl: Mutex<Duration>,
    // Test donations are never stored: they show on the bar for test_ttl, whatever the goal dates.
    test_amounts: Mutex<Vec<f64>>,
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

impl Hub {
    pub fn new(store: Store) -> Hub {
        let (tx, _) = broadcast::channel(64);
        Hub {
            inner: Arc::new(Inner {
                store,
                tx,
                test_ttl: Mutex::new(DEFAULT_TEST_TTL),
                test_amounts: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn set_test_ttl(&self, ttl: Duration) {
        *self.inner.test_ttl.lock().unwrap() = ttl;
    }

    pub fn style(&self) -> anyhow::Result<Value> {
        let saved = self.inner.store.get_setting("style")?.unwrap_or_else(|| json!({}));
        Ok(merge_style(&default_style(), &saved))
    }

    pub fn snapshot(&self) -> anyhow::Result<Value> {
        let store = &self.inner.store;
        let Some(goal) = store.active_goal()? else {
            return Ok(json!({"goal": null, "collected_rub": 0, "style": self.style()?}));
        };
        let test: f64 = self.inner.test_amounts.lock().unwrap().iter().sum();
        Ok(json!({
            "goal": {
                "id": goal.id,
                "title": goal.title,
                "target_rub": goal.target_rub,
                "started_at": to_iso(&goal.started_at),
                "ends_at": goal.ends_at.map(|d| to_iso(&d)),
                "manual_adjust_rub": goal.manual_adjust_rub,
            },
            "collected_rub": round2(store.collected(&goal)? + test),
            "style": self.style()?,
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.inner.tx.subscribe()
    }

    /// Push the current snapshot to every connected overlay; closed connections simply stop receiving.
    pub fn broadcast(&self) {
        match self.snapshot() {
            Ok(snap) => {
                let _ = self.inner.tx.send(snap); // Err only means nobody is listening
            }
            Err(e) => tracing::warn!("snapshot failed: {e:#}"),
        }
    }

    /// Callback for collectors: called after a new donation is stored.
    pub fn notifier(&self) -> Arc<dyn Fn() + Send + Sync> {
        let hub = self.clone();
        Arc::new(move || hub.broadcast())
    }

    pub fn add_test_amount(&self, amount_rub: f64) {
        self.inner.test_amounts.lock().unwrap().push(amount_rub);
        self.broadcast();
        let hub = self.clone();
        let ttl = *self.inner.test_ttl.lock().unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(ttl).await;
            {
                let mut amounts = hub.inner.test_amounts.lock().unwrap();
                if let Some(i) = amounts.iter().position(|a| *a == amount_rub) {
                    amounts.remove(i);
                }
            }
            hub.broadcast();
        });
    }
}
