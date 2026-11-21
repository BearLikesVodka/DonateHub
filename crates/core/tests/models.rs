use chrono::{FixedOffset, TimeZone, Utc};
use donatehub_core::models::{from_iso, to_iso};

#[test]
fn to_iso_matches_python() {
    let t = Utc.with_ymd_and_hms(2026, 11, 21, 14, 0, 0).unwrap();
    assert_eq!(to_iso(&t), "2026-11-21T14:00:00.000000+00:00");
    let msk = FixedOffset::east_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 11, 21, 17, 0, 0).unwrap();
    assert_eq!(to_iso(&msk.with_timezone(&Utc)), "2026-11-21T14:00:00.000000+00:00");
    let micros = Utc.with_ymd_and_hms(2026, 11, 21, 13, 35, 42).unwrap() + chrono::Duration::microseconds(665816);
    assert_eq!(to_iso(&micros), "2026-11-21T13:35:42.665816+00:00");
}

#[test]
fn from_iso_accepts_offsets() {
    let t = Utc.with_ymd_and_hms(2026, 11, 21, 14, 0, 0).unwrap();
    assert_eq!(from_iso("2026-11-21T17:00:00+03:00").unwrap(), t);
    assert_eq!(from_iso("2026-11-21T14:00:00.000000+00:00").unwrap(), t);
    assert_eq!(from_iso("2026-11-21T14:00:00Z").unwrap(), t);
    assert!(from_iso("2026-11-21T14:00:00").is_err());
}
