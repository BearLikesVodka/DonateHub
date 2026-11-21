#![allow(dead_code)]
use chrono::{DateTime, TimeZone, Utc};
use donatehub_core::models::{Donation, Platform};
use serde_json::json;

pub fn t(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, h, m, 0).unwrap()
}

pub fn make_donation(platform: Platform, id: &str, rub: Option<f64>, at: DateTime<Utc>) -> Donation {
    Donation {
        platform,
        external_id: id.into(),
        username: "viewer".into(),
        amount: rub.unwrap_or(0.0),
        currency: "RUB".into(),
        amount_rub: rub,
        created_at: at,
        is_test: false,
        raw: json!({}),
    }
}
