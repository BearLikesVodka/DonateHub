use chrono::{DateTime, Utc};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Da,
    Dx,
    Manual,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Da => "da",
            Platform::Dx => "dx",
            Platform::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Option<Platform> {
        match s {
            "da" => Some(Platform::Da),
            "dx" => Some(Platform::Dx),
            "manual" => Some(Platform::Manual),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Donation {
    pub platform: Platform,
    pub external_id: String,
    pub username: String,
    pub amount: f64,
    pub currency: String,
    pub amount_rub: Option<f64>,
    pub created_at: DateTime<Utc>,
    pub is_test: bool,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub id: i64,
    pub title: String,
    pub target_rub: f64,
    pub started_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub manual_adjust_rub: f64,
}

/// The single UTC string format stored in the DB (same as the Python version), so string comparison orders correctly.
pub fn to_iso(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
}

pub fn from_iso(s: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}
