use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::{DateTime, Utc};
use donatehub_core::app::{build, build_with};
use donatehub_core::collectors::manager::{BoxFut, Collector, Endpoints};
use donatehub_core::collectors::base::MarkOk;
use donatehub_core::store::Store;
use donatehub_core::updates::{UpdateInfo, Updater};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Resp {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }
}

async fn req(app: &Router, method: &str, uri: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Resp {
    let mut b = Request::builder().method(method).uri(uri);
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        b = b.header("host", "127.0.0.1:8000");
    }
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let r = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = r.status();
    let headers = r.headers().clone();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    Resp { status, headers, text: String::from_utf8_lossy(&bytes).into_owned() }
}

async fn get(app: &Router, uri: &str) -> Resp {
    req(app, "GET", uri, None, &[]).await
}

fn app() -> Router {
    build(Store::memory(), false, 47321).0
}

/// Real server for websocket tests.
async fn serve(app: Router) -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    port
}

async fn ws_next(ws: &mut (impl StreamExt<Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>> + Unpin)) -> Value {
    let m = tokio::time::timeout(Duration::from_secs(3), ws.next()).await.unwrap().unwrap().unwrap();
    serde_json::from_str(m.to_text().unwrap()).unwrap()
}

#[tokio::test]
async fn test_ws_receives_update_after_debug_donation() {
    let app = app();
    assert_eq!(req(&app, "POST", "/api/goal", Some(json!({"title": "Микрофон", "target_rub": 10000})), &[]).await.status, 200);
    let port = serve(app.clone()).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
    let first = ws_next(&mut ws).await;
    assert_eq!(first["collected_rub"], json!(0.0));
    assert_eq!(first["goal"]["title"], "Микрофон");
    assert_eq!(req(&app, "POST", "/api/debug/donation", Some(json!({"amount_rub": 500})), &[]).await.status, 200);
    assert_eq!(ws_next(&mut ws).await["collected_rub"], json!(500.0));
}

#[tokio::test]
async fn test_test_donation_ignores_goal_start_and_expires() {
    let (app, state) = build(Store::memory(), false, 47321);
    state.hub.set_test_ttl(Duration::from_millis(200));
    let body = json!({"title": "Микрофон", "target_rub": 10000, "started_at": "2099-01-01T00:00:00+00:00"});
    assert_eq!(req(&app, "POST", "/api/goal", Some(body), &[]).await.status, 200);
    let port = serve(app.clone()).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
    assert_eq!(ws_next(&mut ws).await["collected_rub"], json!(0.0));
    let r = req(&app, "POST", "/api/debug/donation", Some(json!({"amount_rub": 100})), &[]).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["collected_rub"], json!(100.0));
    assert_eq!(ws_next(&mut ws).await["collected_rub"], json!(100.0));
    assert_eq!(ws_next(&mut ws).await["collected_rub"], json!(0.0)); // pushed again once the test amount expires
    let goal = state.store.active_goal().unwrap().unwrap();
    assert_eq!(state.store.collected(&goal).unwrap(), 0.0);
}

#[tokio::test]
async fn debug_donation_validation() {
    let app = app();
    assert_eq!(req(&app, "POST", "/api/debug/donation", Some(json!({"amount_rub": 0})), &[]).await.status, 422);
    assert_eq!(req(&app, "POST", "/api/debug/donation", Some(json!({"amount_rub": 2_000_000})), &[]).await.status, 422);
}

#[tokio::test]
async fn test_style_merge_rejects_unknown_and_bad_types() {
    let app = app();
    let r = req(&app, "PUT", "/api/style", Some(json!({"bar": {"height_vw": "x", "nope": 1, "radius_vw": 2}})), &[]).await;
    assert_eq!(r.status, 200);
    let style = get(&app, "/api/state").await.json()["style"].clone();
    assert_eq!(style["bar"]["height_vw"], json!(7.0));
    assert_eq!(style["bar"]["radius_vw"], json!(2));
    assert!(style["bar"].get("nope").is_none());
    assert_eq!(req(&app, "PUT", "/api/style", Some(json!([1])), &[]).await.status, 422);
}

#[tokio::test]
async fn test_tokens_not_exposed() {
    let app = app();
    assert_eq!(req(&app, "PUT", "/api/platforms/dx", Some(json!({"token": "SECRET"})), &[]).await.status, 200);
    let da = json!({"client_id": "7", "client_secret": "SECRET2"});
    assert_eq!(req(&app, "PUT", "/api/platforms/da", Some(da), &[]).await.status, 200);
    let r = get(&app, "/api/state").await;
    assert!(!r.text.contains("SECRET"));
    let s = r.json();
    assert_eq!(s["platforms"]["dx"]["configured"], true);
    assert_eq!(s["platforms"]["da"]["configured"], false);
    assert_eq!(s["platforms"]["da"]["has_client"], true);
    let port = serve(app).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
    assert!(!ws_next(&mut ws).await.to_string().contains("SECRET"));
}

#[tokio::test]
async fn test_goal_lifecycle() {
    let app = app();
    assert_eq!(req(&app, "POST", "/api/goal", Some(json!({"title": "A", "target_rub": 1000})), &[]).await.status, 200);
    let r = req(&app, "POST", "/api/goal", Some(json!({"title": "B", "target_rub": 1000})), &[]).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["detail"], "уже есть активная цель");
    let r = req(&app, "PATCH", "/api/goal", Some(json!({"manual_adjust_rub": 150, "title": "A2"})), &[]).await;
    assert_eq!(r.status, 200);
    let s = get(&app, "/api/state").await.json();
    assert_eq!(s["collected_rub"], json!(150.0));
    assert_eq!(s["goal"]["title"], "A2");
    assert_eq!(req(&app, "POST", "/api/goal/close", None, &[]).await.status, 200);
    let h = get(&app, "/api/history").await.json();
    assert_eq!(h.as_array().unwrap().len(), 1);
    assert_eq!(h[0]["title"], "A2");
    assert_eq!(h[0]["collected_rub"], json!(150.0));
    let keys: Vec<_> = h[0].as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["id", "title", "target_rub", "collected_rub", "started_at", "closed_at"]);
    assert_eq!(get(&app, "/api/state").await.json()["goal"], Value::Null);
}

#[tokio::test]
async fn test_goal_with_explicit_dates() {
    let app = app();
    let body = json!({"title": "A", "target_rub": 1000, "started_at": "2026-11-20T10:00:00+03:00", "ends_at": "2026-11-30T10:00:00Z"});
    assert_eq!(req(&app, "POST", "/api/goal", Some(body), &[]).await.status, 200);
    let g = get(&app, "/api/state").await.json()["goal"].clone();
    assert!(g["started_at"].as_str().unwrap().starts_with("2026-11-20T07:00:00"));
    assert_eq!(req(&app, "PATCH", "/api/goal", Some(json!({"ends_at": null})), &[]).await.status, 200);
    assert_eq!(get(&app, "/api/state").await.json()["goal"]["ends_at"], Value::Null);
}

#[tokio::test]
async fn test_goal_validation() {
    let app = app();
    assert_eq!(req(&app, "POST", "/api/goal", Some(json!({"title": "", "target_rub": 1000})), &[]).await.status, 422);
    assert_eq!(req(&app, "POST", "/api/goal", Some(json!({"title": "A", "target_rub": 0})), &[]).await.status, 422);
    let naive = json!({"title": "A", "target_rub": 1, "started_at": "2026-11-20T10:00:00"});
    assert_eq!(req(&app, "POST", "/api/goal", Some(naive), &[]).await.status, 422);
    assert_eq!(req(&app, "PATCH", "/api/goal", Some(json!({"title": "x"})), &[]).await.status, 404);
    req(&app, "POST", "/api/goal", Some(json!({"title": "A", "target_rub": 1})), &[]).await;
    let r = req(&app, "PATCH", "/api/goal", Some(json!({"title": null})), &[]).await;
    assert_eq!(r.status, 422);
    assert_eq!(r.json()["detail"], "поле не может быть пустым");
}

#[tokio::test]
async fn errors_have_detail() {
    let app = app();
    let r = req(&app, "POST", "/api/goal", Some(json!({"title": "A", "target_rub": -1})), &[]).await;
    assert_eq!(r.status, 422);
    assert!(r.json()["detail"].is_array());
    let r = req(&app, "POST", "/api/goal", Some(json!("garbage")), &[]).await;
    assert_eq!(r.status, 422);
    assert!(r.json()["detail"].is_array());
    req(&app, "POST", "/api/goal", Some(json!({"title": "A", "target_rub": 1})), &[]).await;
    let r = req(&app, "POST", "/api/goal", Some(json!({"title": "B", "target_rub": 1})), &[]).await;
    assert_eq!(r.status, 409);
    assert!(r.json()["detail"].is_string());
}

#[tokio::test]
async fn test_pages_served() {
    let app = app();
    let r = get(&app, "/overlay").await;
    assert_eq!(r.status, 200);
    assert!(r.headers["content-type"].to_str().unwrap().starts_with("text/html"));
    assert_eq!(get(&app, "/").await.status, 200);
    let js = get(&app, "/static/admin.js").await;
    assert_eq!(js.status, 200);
    assert!(js.headers["content-type"].to_str().unwrap().contains("charset=utf-8"));
    assert_eq!(get(&app, "/static/nope.js").await.status, 404);
    assert!(get(&app, "/api/presets").await.json().as_array().unwrap().len() >= 5);
}

#[tokio::test]
async fn test_da_start_requires_client() {
    let app = app();
    assert_eq!(get(&app, "/auth/da/start").await.status, 400);
    req(&app, "PUT", "/api/platforms/da", Some(json!({"client_id": "7", "client_secret": "s"})), &[]).await;
    let r = get(&app, "/auth/da/start").await;
    assert!(r.status == 302 || r.status == 307);
    assert!(r.headers["location"].to_str().unwrap().starts_with("https://www.donationalerts.com/oauth/authorize"));
}

#[tokio::test]
async fn test_da_callback_rejects_bad_state() {
    let app = app();
    req(&app, "PUT", "/api/platforms/da", Some(json!({"client_id": "7", "client_secret": "s"})), &[]).await;
    assert_eq!(get(&app, "/auth/da/callback?code=abc&state=forged").await.status, 400);
}

#[tokio::test]
async fn test_cross_site_post_rejected() {
    let app = app();
    req(&app, "POST", "/api/goal", Some(json!({"title": "A", "target_rub": 1000})), &[]).await;
    let r = req(&app, "POST", "/api/goal/close", None, &[("origin", "https://evil.example")]).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["detail"], "запрос с чужого сайта отклонён");
    assert!(get(&app, "/api/state").await.json()["goal"].is_object());
    assert_eq!(req(&app, "POST", "/api/goal/close", None, &[("origin", "http://127.0.0.1:8000")]).await.status, 200);
}

#[tokio::test]
async fn test_foreign_host_rejected() {
    let app = app();
    assert_eq!(req(&app, "GET", "/api/state", None, &[("host", "rebind.evil.example")]).await.status, 400);
    assert_eq!(req(&app, "GET", "/api/state", None, &[("host", "localhost:8000")]).await.status, 200);
    assert_eq!(req(&app, "GET", "/api/state", None, &[("host", "127.0.0.1:8000")]).await.status, 200);
}

#[tokio::test]
async fn host_with_port_allowed() {
    let app = app();
    for host in ["127.0.0.1:47321", "localhost:47321", "127.0.0.1", "localhost"] {
        assert_eq!(req(&app, "GET", "/api/state", None, &[("host", host)]).await.status, 200, "{host}");
    }
    assert_eq!(req(&app, "GET", "/api/state", None, &[("host", "127.0.0.1.evil.example:47321")]).await.status, 400);
}

struct FakeCollector {
    calls: Arc<Mutex<Vec<DateTime<Utc>>>>,
}

impl Collector for FakeCollector {
    fn catch_up(&self, since: DateTime<Utc>) -> BoxFut<'_, anyhow::Result<usize>> {
        self.calls.lock().unwrap().push(since);
        Box::pin(async { Ok(0) })
    }
    fn connect(&self, _mark_ok: MarkOk) -> BoxFut<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn test_backdated_goal_triggers_backfill() {
    let (app, state) = build(Store::memory(), false, 47321);
    let calls = Arc::new(Mutex::new(Vec::new()));
    state.collectors.set_instance("dx", Arc::new(FakeCollector { calls: calls.clone() }));
    let body = json!({"title": "A", "target_rub": 1000, "started_at": "2026-11-01T00:00:00Z"});
    req(&app, "POST", "/api/goal", Some(body), &[]).await;
    req(&app, "PATCH", "/api/goal", Some(json!({"started_at": "2026-10-01T00:00:00Z"})), &[]).await;
    req(&app, "PATCH", "/api/goal", Some(json!({"title": "B"})), &[]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let got: Vec<String> = calls.lock().unwrap().iter().map(|d| d.to_rfc3339()).collect();
    assert_eq!(got, ["2026-11-01T00:00:00+00:00", "2026-10-01T00:00:00+00:00"]);
}

#[tokio::test]
async fn test_redirect_uri_follows_port() {
    let (app, _) = build(Store::memory(), false, 50001);
    let s = get(&app, "/api/state").await.json();
    assert_eq!(s["platforms"]["da"]["redirect_uri"], "http://127.0.0.1:50001/auth/da/callback");
    assert_eq!(s["port"]["current"], 50001);
    req(&app, "PUT", "/api/platforms/da", Some(json!({"client_id": "7", "client_secret": "s"})), &[]).await;
    let loc = get(&app, "/auth/da/start").await.headers["location"].to_str().unwrap().to_string();
    assert!(loc.contains("127.0.0.1%3A50001%2Fauth%2Fda%2Fcallback"), "{loc}");
}

#[tokio::test]
async fn test_port_setting_validation() {
    let app = app();
    assert_eq!(req(&app, "PUT", "/api/settings/port", Some(json!({"port": 70000})), &[]).await.status, 422);
    assert_eq!(req(&app, "PUT", "/api/settings/port", Some(json!({"port": 80})), &[]).await.status, 422);
    assert_eq!(req(&app, "PUT", "/api/settings/port", Some(json!({"port": 48000})), &[]).await.status, 200);
    let port = get(&app, "/api/state").await.json()["port"].clone();
    assert_eq!((port["saved"].clone(), port["current"].clone()), (json!(48000), json!(47321)));
}

#[tokio::test]
async fn test_da_callback_success_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "AT", "refresh_token": "RT", "expires_in": 3600})))
        .mount(&server)
        .await;
    let endpoints = Endpoints { da_site: server.uri(), ..Endpoints::default() };
    let (app, state) = build_with(Store::memory(), false, 47321, endpoints);
    req(&app, "PUT", "/api/platforms/da", Some(json!({"client_id": "7", "client_secret": "s"})), &[]).await;
    let loc = get(&app, "/auth/da/start").await.headers["location"].to_str().unwrap().to_string();
    let st = loc.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string();
    let r = get(&app, &format!("/auth/da/callback?code=abc&state={st}")).await;
    assert_eq!(r.status, 200);
    assert!(r.headers["content-type"].to_str().unwrap().starts_with("text/html"));
    assert!(r.text.contains("подключён"));
    assert_eq!(get(&app, "/api/state").await.json()["platforms"]["da"]["configured"], true);
    let form = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
    assert!(form.contains("code=abc") && form.contains("47321%2Fauth%2Fda%2Fcallback"), "{form}");
    assert_eq!(state.store.get_setting("da").unwrap().unwrap()["client_secret"], "s");
    // the state is single-use
    assert_eq!(get(&app, &format!("/auth/da/callback?code=abc&state={st}")).await.status, 400);
}

// ---------- updates ----------

struct FakeUpdater {
    info: Mutex<UpdateInfo>,
    install_result: Result<(), String>,
    installs: Mutex<u32>,
}

impl Updater for FakeUpdater {
    fn info(&self) -> UpdateInfo {
        self.info.lock().unwrap().clone()
    }
    fn install(&self) -> Result<(), String> {
        *self.installs.lock().unwrap() += 1;
        self.install_result.clone()
    }
}

fn app_with_updater(install_result: Result<(), String>) -> (Router, Arc<FakeUpdater>) {
    let (router, state) = build(Store::memory(), false, 47321);
    let fake = Arc::new(FakeUpdater {
        info: Mutex::new(UpdateInfo { current: "0.3.0".into(), available: Some("0.3.1".into()), installing: false }),
        install_result,
        installs: Mutex::new(0),
    });
    state.set_updater(fake.clone());
    (router, fake)
}

#[tokio::test]
async fn update_unsupported_without_shell() {
    let app = app();
    assert_eq!(get(&app, "/api/update").await.json(), json!({"supported": false}));
    let r = req(&app, "POST", "/api/update/install", None, &[]).await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["detail"], "обновление недоступно в этом режиме");
}

#[tokio::test]
async fn update_info_from_shell() {
    let (app, _) = app_with_updater(Ok(()));
    assert_eq!(
        get(&app, "/api/update").await.json(),
        json!({"supported": true, "current": "0.3.0", "available": "0.3.1", "installing": false})
    );
}

#[tokio::test]
async fn update_install_calls_shell() {
    let (app, fake) = app_with_updater(Ok(()));
    let r = req(&app, "POST", "/api/update/install", None, &[("origin", "http://127.0.0.1:47321")]).await;
    assert_eq!(r.status, 202);
    assert_eq!(r.json(), json!({"ok": true}));
    assert_eq!(*fake.installs.lock().unwrap(), 1);
}

#[tokio::test]
async fn update_install_error_is_409() {
    let (app, _) = app_with_updater(Err("обновлений нет".into()));
    let r = req(&app, "POST", "/api/update/install", None, &[]).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["detail"], "обновлений нет");
}

#[tokio::test]
async fn update_install_cross_site_rejected() {
    let (app, fake) = app_with_updater(Ok(()));
    let r = req(&app, "POST", "/api/update/install", None, &[("origin", "https://evil.example")]).await;
    assert_eq!(r.status, 403);
    let r = req(&app, "POST", "/api/update/install", None, &[("host", "evil.example")]).await;
    assert_eq!(r.status, 400);
    assert_eq!(*fake.installs.lock().unwrap(), 0);
}
