use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use crate::models::{Donation, Goal, Platform, from_iso, to_iso};

/// Byte-for-byte the schema of the Python version, so both read the same `app.db`.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS donations (
    platform TEXT NOT NULL,
    external_id TEXT NOT NULL,
    username TEXT NOT NULL,
    amount REAL NOT NULL,
    currency TEXT NOT NULL,
    amount_rub REAL,
    created_at TEXT NOT NULL,
    is_test INTEGER NOT NULL,
    raw_json TEXT NOT NULL,
    PRIMARY KEY (platform, external_id)
);
CREATE INDEX IF NOT EXISTS donations_created ON donations(created_at);
CREATE TABLE IF NOT EXISTS goals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    target_rub REAL NOT NULL,
    started_at TEXT NOT NULL,
    ends_at TEXT,
    closed_at TEXT,
    manual_adjust_rub REAL NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalExists;

/// Partial goal update. `ends_at: Some(None)` clears the end date.
#[derive(Debug, Clone, Default)]
pub struct GoalPatch {
    pub title: Option<String>,
    pub target_rub: Option<f64>,
    pub started_at: Option<DateTime<Utc>>,
    pub ends_at: Option<Option<DateTime<Utc>>>,
    pub manual_adjust_rub: Option<f64>,
}

#[derive(Clone)]
pub struct Store {
    db: Arc<Mutex<Connection>>,
}

fn opt_iso(dt: Option<DateTime<Utc>>) -> Option<String> {
    dt.map(|d| to_iso(&d))
}

fn opt_dt(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    s.map(|s| from_iso(&s)).transpose()
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        Self::init(Connection::open(path)?)
    }

    pub fn memory() -> Store {
        Self::init(Connection::open_in_memory().expect("in-memory sqlite")).expect("schema")
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.execute_batch(SCHEMA)?;
        Ok(Store { db: Arc::new(Mutex::new(conn)) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    // donations

    pub fn add_donation(&self, d: &Donation) -> Result<bool> {
        let n = self.db().execute(
            "INSERT OR IGNORE INTO donations VALUES (?,?,?,?,?,?,?,?,?)",
            params![
                d.platform.as_str(),
                d.external_id,
                d.username,
                d.amount,
                d.currency,
                d.amount_rub,
                to_iso(&d.created_at),
                d.is_test as i64,
                d.raw.to_string(),
            ],
        )?;
        Ok(n == 1)
    }

    pub fn set_amount_rub(&self, platform: &str, external_id: &str, amount_rub: f64) -> Result<()> {
        self.db().execute(
            "UPDATE donations SET amount_rub=? WHERE platform=? AND external_id=?",
            params![amount_rub, platform, external_id],
        )?;
        Ok(())
    }

    pub fn donations_missing_rub(&self) -> Result<Vec<Donation>> {
        let db = self.db();
        let mut stmt = db.prepare("SELECT * FROM donations WHERE amount_rub IS NULL")?;
        let rows = stmt.query_map([], raw_donation)?;
        rows.map(|r| r?.into_donation()).collect()
    }

    pub fn latest_donation_time(&self, platform: &str) -> Result<Option<DateTime<Utc>>> {
        let s: Option<String> =
            self.db().query_row("SELECT MAX(created_at) FROM donations WHERE platform=?", [platform], |r| r.get(0))?;
        opt_dt(s)
    }

    // goals

    pub fn get_goal(&self, id: i64) -> Result<Option<Goal>> {
        let raw = self.db().query_row("SELECT * FROM goals WHERE id=?", [id], raw_goal).optional()?;
        raw.map(RawGoal::into_goal).transpose()
    }

    pub fn active_goal(&self) -> Result<Option<Goal>> {
        let raw = self
            .db()
            .query_row("SELECT * FROM goals WHERE closed_at IS NULL ORDER BY id DESC LIMIT 1", [], raw_goal)
            .optional()?;
        raw.map(RawGoal::into_goal).transpose()
    }

    pub fn create_goal(
        &self,
        title: &str,
        target_rub: f64,
        started_at: DateTime<Utc>,
        ends_at: Option<DateTime<Utc>>,
    ) -> Result<std::result::Result<Goal, GoalExists>> {
        let id = {
            // Check and insert under one lock: parallel requests must not both see "no active goal".
            let db = self.db();
            let active: i64 = db.query_row("SELECT COUNT(*) FROM goals WHERE closed_at IS NULL", [], |r| r.get(0))?;
            if active > 0 {
                return Ok(Err(GoalExists));
            }
            db.execute(
                "INSERT INTO goals (title, target_rub, started_at, ends_at) VALUES (?,?,?,?)",
                params![title, target_rub, to_iso(&started_at), opt_iso(ends_at)],
            )?;
            db.last_insert_rowid()
        };
        Ok(Ok(self.get_goal(id)?.expect("just inserted")))
    }

    pub fn update_goal(&self, id: i64, patch: &GoalPatch) -> Result<Option<Goal>> {
        {
            let db = self.db();
            let set = |col: &str, v: &dyn rusqlite::ToSql| -> Result<()> {
                db.execute(&format!("UPDATE goals SET {col}=? WHERE id=?"), params![v, id])?;
                Ok(())
            };
            if let Some(v) = &patch.title {
                set("title", v)?;
            }
            if let Some(v) = patch.target_rub {
                set("target_rub", &v)?;
            }
            if let Some(v) = patch.started_at {
                set("started_at", &to_iso(&v))?;
            }
            if let Some(v) = patch.ends_at {
                set("ends_at", &opt_iso(v))?;
            }
            if let Some(v) = patch.manual_adjust_rub {
                set("manual_adjust_rub", &v)?;
            }
        }
        self.get_goal(id)
    }

    pub fn close_goal(&self, id: i64, closed_at: DateTime<Utc>) -> Result<Option<Goal>> {
        self.db().execute("UPDATE goals SET closed_at=? WHERE id=?", params![to_iso(&closed_at), id])?;
        self.get_goal(id)
    }

    pub fn collected(&self, goal: &Goal) -> Result<f64> {
        let end = [goal.closed_at, goal.ends_at].into_iter().flatten().min().map(|d| to_iso(&d));
        let sum: f64 = self.db().query_row(
            "SELECT COALESCE(SUM(amount_rub), 0) FROM donations \
             WHERE is_test=0 AND amount_rub IS NOT NULL AND created_at >= ? AND created_at < ?",
            params![to_iso(&goal.started_at), end.unwrap_or_else(|| "9999".into())],
            |r| r.get(0),
        )?;
        Ok(sum + goal.manual_adjust_rub)
    }

    pub fn history(&self) -> Result<Vec<(Goal, f64)>> {
        let raws: Vec<RawGoal> = {
            let db = self.db();
            let mut stmt =
                db.prepare("SELECT * FROM goals WHERE closed_at IS NOT NULL ORDER BY started_at DESC, id DESC")?;
            stmt.query_map([], raw_goal)?.collect::<rusqlite::Result<_>>()?
        };
        raws.into_iter()
            .map(|r| {
                let g = r.into_goal()?;
                let total = self.collected(&g)?;
                Ok((g, total))
            })
            .collect()
    }

    // settings

    pub fn get_setting(&self, key: &str) -> Result<Option<Value>> {
        let s: Option<String> =
            self.db().query_row("SELECT value_json FROM settings WHERE key=?", [key], |r| r.get(0)).optional()?;
        Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn set_setting(&self, key: &str, value: &Value) -> Result<()> {
        self.db().execute(
            "INSERT INTO settings VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
            params![key, value.to_string()],
        )?;
        Ok(())
    }
}

/// Row values as stored; parsing (dates, JSON) happens outside the rusqlite callback so errors keep their cause.
struct RawDonation {
    platform: String,
    external_id: String,
    username: String,
    amount: f64,
    currency: String,
    amount_rub: Option<f64>,
    created_at: String,
    is_test: i64,
    raw_json: String,
}

fn raw_donation(r: &Row) -> rusqlite::Result<RawDonation> {
    Ok(RawDonation {
        platform: r.get("platform")?,
        external_id: r.get("external_id")?,
        username: r.get("username")?,
        amount: r.get("amount")?,
        currency: r.get("currency")?,
        amount_rub: r.get("amount_rub")?,
        created_at: r.get("created_at")?,
        is_test: r.get("is_test")?,
        raw_json: r.get("raw_json")?,
    })
}

impl RawDonation {
    fn into_donation(self) -> Result<Donation> {
        Ok(Donation {
            platform: Platform::parse(&self.platform)
                .ok_or_else(|| anyhow::anyhow!("unknown platform {:?}", self.platform))?,
            external_id: self.external_id,
            username: self.username,
            amount: self.amount,
            currency: self.currency,
            amount_rub: self.amount_rub,
            created_at: from_iso(&self.created_at)?,
            is_test: self.is_test != 0,
            raw: serde_json::from_str(&self.raw_json)?,
        })
    }
}

struct RawGoal {
    id: i64,
    title: String,
    target_rub: f64,
    started_at: String,
    ends_at: Option<String>,
    closed_at: Option<String>,
    manual_adjust_rub: f64,
}

fn raw_goal(r: &Row) -> rusqlite::Result<RawGoal> {
    Ok(RawGoal {
        id: r.get("id")?,
        title: r.get("title")?,
        target_rub: r.get("target_rub")?,
        started_at: r.get("started_at")?,
        ends_at: r.get("ends_at")?,
        closed_at: r.get("closed_at")?,
        manual_adjust_rub: r.get("manual_adjust_rub")?,
    })
}

impl RawGoal {
    fn into_goal(self) -> Result<Goal> {
        Ok(Goal {
            id: self.id,
            title: self.title,
            target_rub: self.target_rub,
            started_at: from_iso(&self.started_at)?,
            ends_at: opt_dt(self.ends_at)?,
            closed_at: opt_dt(self.closed_at)?,
            manual_adjust_rub: self.manual_adjust_rub,
        })
    }
}
