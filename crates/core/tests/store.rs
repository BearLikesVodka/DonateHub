mod common;

use common::{make_donation, t};
use donatehub_core::config::resolve_port;
use donatehub_core::models::Platform::{Da, Dx};
use donatehub_core::store::{GoalPatch, Store};
use serde_json::json;

#[test]
fn test_add_donation_dedup() {
    let s = Store::memory();
    let d = make_donation(Dx, "a1", Some(100.0), t(12, 0));
    assert!(s.add_donation(&d).unwrap());
    assert!(!s.add_donation(&d).unwrap());
}

#[test]
fn test_same_id_different_platforms_both_kept() {
    let s = Store::memory();
    assert!(s.add_donation(&make_donation(Dx, "1", Some(100.0), t(12, 0))).unwrap());
    assert!(s.add_donation(&make_donation(Da, "1", Some(100.0), t(12, 0))).unwrap());
}

#[test]
fn test_collected_window_and_tests() {
    let s = Store::memory();
    let g = s.create_goal("Микрофон", 10000.0, t(10, 0), None).unwrap().unwrap();
    s.add_donation(&make_donation(Dx, "before", Some(500.0), t(9, 0))).unwrap();
    s.add_donation(&make_donation(Dx, "in", Some(300.0), t(11, 0))).unwrap();
    s.add_donation(&make_donation(Da, "in2", Some(200.0), t(12, 0))).unwrap();
    let mut test = make_donation(Dx, "test", Some(999.0), t(12, 0));
    test.is_test = true;
    s.add_donation(&test).unwrap();
    s.add_donation(&make_donation(Da, "norub", None, t(12, 0))).unwrap();
    assert_eq!(s.collected(&g).unwrap(), 500.0);
    s.update_goal(g.id, &GoalPatch { manual_adjust_rub: Some(50.0), ..Default::default() }).unwrap();
    assert_eq!(s.collected(&s.active_goal().unwrap().unwrap()).unwrap(), 550.0);
}

#[test]
fn test_ends_at_limits_window() {
    let s = Store::memory();
    let g = s.create_goal("A", 1000.0, t(10, 0), Some(t(12, 0))).unwrap().unwrap();
    s.add_donation(&make_donation(Dx, "in", Some(100.0), t(11, 0))).unwrap();
    s.add_donation(&make_donation(Dx, "after", Some(100.0), t(13, 0))).unwrap();
    assert_eq!(s.collected(&g).unwrap(), 100.0);
}

#[test]
fn test_closed_goal_window_and_history() {
    let s = Store::memory();
    let g = s.create_goal("A", 1000.0, t(10, 0), None).unwrap().unwrap();
    s.add_donation(&make_donation(Dx, "x", Some(100.0), t(11, 0))).unwrap();
    s.close_goal(g.id, t(12, 0)).unwrap();
    s.add_donation(&make_donation(Dx, "y", Some(100.0), t(13, 0))).unwrap();
    assert!(s.active_goal().unwrap().is_none());
    let h = s.history().unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].0.title, "A");
    assert_eq!(h[0].1, 100.0);
}

#[test]
fn test_single_active_goal() {
    let s = Store::memory();
    assert!(s.create_goal("A", 1.0, t(1, 0), None).unwrap().is_ok());
    assert!(s.create_goal("B", 1.0, t(2, 0), None).unwrap().is_err());
}

#[test]
fn test_settings_roundtrip() {
    let s = Store::memory();
    s.set_setting("style", &json!({"a": 1})).unwrap();
    assert_eq!(s.get_setting("style").unwrap(), Some(json!({"a": 1})));
    assert_eq!(s.get_setting("missing").unwrap(), None);
}

#[test]
fn test_missing_rub_and_set_amount() {
    let s = Store::memory();
    let mut d = make_donation(Da, "u1", None, t(12, 0));
    d.amount = 2.0;
    d.currency = "USD".into();
    s.add_donation(&d).unwrap();
    let missing = s.donations_missing_rub().unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].currency, "USD");
    assert_eq!(missing[0].amount, 2.0);
    s.set_amount_rub("da", "u1", 180.0).unwrap();
    assert!(s.donations_missing_rub().unwrap().is_empty());
}

#[test]
fn test_latest_donation_time() {
    let s = Store::memory();
    assert_eq!(s.latest_donation_time("dx").unwrap(), None);
    s.add_donation(&make_donation(Dx, "a", Some(100.0), t(9, 0))).unwrap();
    s.add_donation(&make_donation(Dx, "b", Some(100.0), t(11, 0))).unwrap();
    s.add_donation(&make_donation(Da, "c", Some(100.0), t(15, 0))).unwrap();
    assert_eq!(s.latest_donation_time("dx").unwrap(), Some(t(11, 0)));
}

#[test]
fn test_resolve_port_priority() {
    let s = Store::memory();
    let none = |_: &str| None;
    assert_eq!(resolve_port(&s, &none), 47321);
    s.set_setting("port", &json!(48000)).unwrap();
    assert_eq!(resolve_port(&s, &none), 48000);
    assert_eq!(resolve_port(&s, &|_: &str| Some("49000".into())), 49000);
    assert_eq!(resolve_port(&s, &|_: &str| Some("nope".into())), 48000);
    assert_eq!(resolve_port(&s, &|_: &str| Some("80".into())), 48000);
    s.set_setting("port", &json!("garbage")).unwrap();
    assert_eq!(resolve_port(&s, &none), 47321);
}

#[test]
fn store_reads_python_written_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("app.db");
    drop(Store::open(&path).unwrap()); // creates the schema, as the Python version does
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
        "INSERT INTO goals (title, target_rub, started_at, ends_at, closed_at, manual_adjust_rub) \
         VALUES ('Цель', 9000, '2026-11-21T14:00:00.000000+00:00', NULL, NULL, 0);
         INSERT INTO donations VALUES ('dx','p1','v',100,'RUB',100.0,'2026-11-21T14:30:00.123456+00:00',0,'{\"id\":\"p1\"}');
         INSERT INTO donations VALUES ('da','p2','v',2,'USD',NULL,'2026-11-21T14:40:00.000000+00:00',0,'{}');
         INSERT INTO settings VALUES ('da', '{\"client_id\":\"x\",\"expires_at\":1795267310.5}');",
    )
    .unwrap();
    let s = Store::open(&path).unwrap();
    s.add_donation(&make_donation(Dx, "r1", Some(50.0), t(14, 15))).unwrap();
    let g = s.active_goal().unwrap().unwrap();
    assert_eq!(g.started_at, t(14, 0));
    assert_eq!(s.collected(&g).unwrap(), 150.0);
    assert_eq!(s.latest_donation_time("dx").unwrap().unwrap().to_rfc3339(), "2026-11-21T14:30:00.123456+00:00");
    assert_eq!(s.donations_missing_rub().unwrap().len(), 1);
    assert_eq!(s.get_setting("da").unwrap().unwrap()["expires_at"], json!(1795267310.5));
}

#[test]
fn concurrent_create_goal_makes_one_goal() {
    // A double-click on "create" sends two requests at once; axum handles them in parallel.
    for _ in 0..50 {
        let s = Store::memory();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let (s, b) = (s.clone(), barrier.clone());
                std::thread::spawn(move || {
                    b.wait();
                    s.create_goal(&format!("G{i}"), 1.0, t(10, 0), None).unwrap().is_ok()
                })
            })
            .collect();
        let created = handles.into_iter().map(|h| h.join().unwrap()).filter(|ok| *ok).count();
        assert_eq!(created, 1);
    }
}
