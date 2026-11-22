//! HTTP API, pages and the overlay WebSocket — the same routes and JSON as the Python (FastAPI) version.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use crate::assets;
use crate::collectors::base::describe_error;
use crate::collectors::donationalerts as da;
use crate::collectors::manager::{Collectors, Endpoints};
use crate::config::{resolve_port, valid_port_json};
use crate::hub::Hub;
use crate::models::{from_iso, to_iso};
use crate::store::{GoalPatch, Store};
use crate::style::{load_presets, merge_style};
use crate::updates::Updater;

const LOCAL_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];

const DA_DONE_PAGE: &str = r#"<!doctype html><html lang="ru"><meta charset="utf-8"><title>DonationAlerts подключён</title>
<body style="font-family:system-ui,sans-serif;background:#1c1c1e;color:#ececee;display:grid;place-items:center;height:100vh;margin:0">
<div style="text-align:center"><h1 style="font-weight:600">DonationAlerts подключён</h1>
<p style="color:#9d9da4">Вкладку можно закрыть и вернуться в приложение.</p></div></body></html>"#;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub hub: Hub,
    pub collectors: Collectors,
    pub port: u16,
    oauth_states: Arc<Mutex<HashSet<String>>>,
    updater: Arc<OnceLock<Arc<dyn Updater>>>,
}

impl AppState {
    /// Registered by the desktop shell; without it (headless, tests) updates are reported as unsupported.
    pub fn set_updater(&self, updater: Arc<dyn Updater>) {
        let _ = self.updater.set(updater);
    }
}

/// Router + shared state. Must run inside a tokio runtime when `start_collectors` is true.
pub fn build(store: Store, start_collectors: bool, port: u16) -> (Router, AppState) {
    build_with(store, start_collectors, port, Endpoints::default())
}

pub fn build_with(store: Store, start_collectors: bool, port: u16, endpoints: Endpoints) -> (Router, AppState) {
    let hub = Hub::new(store.clone());
    let collectors = Collectors::new(store.clone(), hub.clone(), port, endpoints);
    if start_collectors {
        collectors.start();
    }
    let state = AppState { store, hub, collectors, port, oauth_states: Arc::default(), updater: Arc::default() };
    (router(state.clone()), state)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| page("admin.html")))
        .route("/overlay", get(|| page("overlay.html")))
        .route("/static/{*path}", get(|Path(p): Path<String>| page_owned(p)))
        .route("/ws", get(ws_endpoint))
        .route("/api/state", get(api_state))
        .route("/api/settings/port", put(set_port))
        .route("/api/presets", get(|| async { Json(Value::Array(load_presets())) }))
        .route("/api/style", put(put_style))
        .route("/api/goal", post(create_goal).patch(patch_goal))
        .route("/api/goal/close", post(close_goal))
        .route("/api/history", get(history))
        .route("/api/debug/donation", post(debug_donation))
        .route("/api/platforms/dx", put(set_dx))
        .route("/api/platforms/da", put(set_da))
        .route("/api/update", get(update_info))
        .route("/api/update/install", post(update_install))
        .route("/auth/da/start", get(da_start))
        .route("/auth/da/callback", get(da_callback))
        .layer(middleware::from_fn(same_origin_writes))
        .layer(middleware::from_fn(trusted_host))
        .with_state(state)
}

// ---------- errors ----------

pub struct ApiError(StatusCode, Value);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"detail": self.1}))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("request failed: {e:#}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!("внутренняя ошибка"))
    }
}

type ApiResult<T = Json<Value>> = Result<T, ApiError>;

fn fail(code: StatusCode, msg: impl Into<String>) -> ApiError {
    ApiError(code, json!(msg.into()))
}

/// Body validation error, shaped like FastAPI's (a list), which the settings page shows as "check the fields".
fn invalid(field: &str, msg: &str) -> ApiError {
    let loc = if field.is_empty() { json!(["body"]) } else { json!(["body", field]) };
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, json!([{"type": "value_error", "loc": loc, "msg": msg}]))
}

// ---------- body parsing ----------

fn body_object(bytes: &Bytes) -> Result<Map<String, Value>, ApiError> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(invalid("", "Input should be a valid dictionary")),
        Err(_) => Err(invalid("", "JSON decode error")),
    }
}

fn required<'a>(m: &'a Map<String, Value>, f: &str) -> Result<&'a Value, ApiError> {
    m.get(f).ok_or_else(|| invalid(f, "Field required"))
}

fn as_str(v: &Value, f: &str, min: usize, max: usize) -> Result<String, ApiError> {
    let s = v.as_str().ok_or_else(|| invalid(f, "Input should be a valid string"))?;
    let n = s.chars().count();
    if n < min || n > max {
        return Err(invalid(f, &format!("String should have {min}..{max} characters")));
    }
    Ok(s.to_string())
}

fn as_num(v: &Value, f: &str) -> Result<f64, ApiError> {
    v.as_f64().filter(|_| v.is_number()).ok_or_else(|| invalid(f, "Input should be a valid number"))
}

fn as_positive(v: &Value, f: &str) -> Result<f64, ApiError> {
    let x = as_num(v, f)?;
    if x > 0.0 { Ok(x) } else { Err(invalid(f, "Input should be greater than 0")) }
}

/// Timezone-aware datetime (pydantic AwareDatetime): naive strings are rejected.
fn as_dt(v: &Value, f: &str) -> Result<DateTime<Utc>, ApiError> {
    let s = v.as_str().ok_or_else(|| invalid(f, "Input should be a valid datetime"))?;
    from_iso(s).map_err(|_| invalid(f, "Input should be a valid datetime with timezone"))
}

fn opt_dt(m: &Map<String, Value>, f: &str) -> Result<Option<DateTime<Utc>>, ApiError> {
    match m.get(f) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => as_dt(v, f).map(Some),
    }
}

// ---------- middleware ----------

async fn trusted_host(req: Request, next: Next) -> Response {
    // Blocks DNS rebinding: a foreign domain resolving to 127.0.0.1 still sends its own Host header.
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().authority().map(|a| a.to_string()))
        .unwrap_or_default();
    let name = host.split(':').next().unwrap_or("");
    if !LOCAL_HOSTS.contains(&name) {
        return (StatusCode::BAD_REQUEST, "Invalid host header").into_response();
    }
    next.run(req).await
}

async fn same_origin_writes(req: Request, next: Next) -> Response {
    // Any website open in the streamer's browser can POST to localhost; only our own pages may change state.
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && let Some(origin) = req.headers().get(header::ORIGIN) {
        let ok = origin
            .to_str()
            .ok()
            .and_then(|o| url::Url::parse(o).ok())
            .and_then(|u| u.host_str().map(str::to_string))
            .is_some_and(|h| LOCAL_HOSTS.contains(&h.as_str()));
        if !ok {
            return fail(StatusCode::FORBIDDEN, "запрос с чужого сайта отклонён").into_response();
        }
    }
    next.run(req).await
}

// ---------- pages ----------

async fn page(path: &'static str) -> Response {
    page_owned(path.to_string()).await
}

async fn page_owned(path: String) -> Response {
    let Some((body, mime)) = assets::get(&path) else {
        return fail(StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    let textual = mime.starts_with("text/") || mime.contains("javascript") || mime.contains("json");
    let ct = if textual { format!("{mime}; charset=utf-8") } else { mime };
    let ct = HeaderValue::from_str(&ct).unwrap_or(HeaderValue::from_static("application/octet-stream"));
    ([(header::CONTENT_TYPE, ct)], body.into_owned()).into_response()
}

// ---------- websocket ----------

async fn ws_endpoint(State(st): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| overlay_socket(st.hub, socket))
}

async fn overlay_socket(hub: Hub, mut socket: WebSocket) {
    let mut rx = hub.subscribe();
    let send = |v: &Value| Message::Text(v.to_string().into());
    match hub.snapshot() {
        Ok(snap) => {
            if socket.send(send(&snap)).await.is_err() {
                return;
            }
        }
        Err(e) => {
            tracing::warn!("snapshot failed: {e:#}");
            return;
        }
    }
    loop {
        tokio::select! {
            msg = rx.recv() => {
                let snap = match msg {
                    Ok(s) => s,
                    // Fell behind: the latest state is all an overlay needs.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => match hub.snapshot() {
                        Ok(s) => s,
                        Err(_) => continue,
                    },
                    Err(_) => return,
                };
                if socket.send(send(&snap)).await.is_err() {
                    return;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {} // overlays never send anything meaningful
            },
        }
    }
}

// ---------- api ----------

fn port_info(st: &AppState) -> Value {
    json!({"current": st.port, "saved": resolve_port(&st.store, &|k| std::env::var(k).ok())})
}

async fn api_state(State(st): State<AppState>) -> ApiResult {
    let mut snap = st.hub.snapshot()?;
    snap["platforms"] = st.collectors.platforms();
    snap["port"] = port_info(&st);
    Ok(Json(snap))
}

async fn set_port(State(st): State<AppState>, body: Bytes) -> ApiResult {
    let m = body_object(&body)?;
    let v = required(&m, "port")?;
    let is_int = v.as_f64().is_some_and(|x| x.fract() == 0.0) && v.is_number();
    let port = is_int
        .then(|| valid_port_json(&json!(v.as_f64().unwrap() as i64)))
        .flatten()
        .ok_or_else(|| invalid("port", "Input should be an integer between 1024 and 65535"))?;
    st.store.set_setting("port", &json!(port))?;
    Ok(Json(port_info(&st)))
}

async fn put_style(State(st): State<AppState>, body: Bytes) -> ApiResult {
    let patch = Value::Object(body_object(&body)?);
    let style = merge_style(&st.hub.style()?, &patch);
    st.store.set_setting("style", &style)?;
    st.hub.broadcast();
    Ok(Json(style))
}

fn require_goal(st: &AppState) -> Result<crate::models::Goal, ApiError> {
    st.store.active_goal()?.ok_or_else(|| fail(StatusCode::NOT_FOUND, "нет активной цели"))
}

async fn create_goal(State(st): State<AppState>, body: Bytes) -> ApiResult {
    let m = body_object(&body)?;
    let title = as_str(required(&m, "title")?, "title", 1, 200)?;
    let target = as_positive(required(&m, "target_rub")?, "target_rub")?;
    let started = opt_dt(&m, "started_at")?.unwrap_or_else(Utc::now);
    let ends = opt_dt(&m, "ends_at")?;
    let goal = st
        .store
        .create_goal(&title, target, started, ends)?
        .map_err(|_| fail(StatusCode::CONFLICT, "уже есть активная цель"))?;
    st.collectors.backfill(goal.started_at);
    st.hub.broadcast();
    Ok(Json(st.hub.snapshot()?))
}

async fn patch_goal(State(st): State<AppState>, body: Bytes) -> ApiResult {
    let m = body_object(&body)?;
    let mut patch = GoalPatch::default();
    let mut has_null = false;
    for (key, v) in &m {
        if v.is_null() {
            if key == "ends_at" {
                patch.ends_at = Some(None);
            } else if matches!(key.as_str(), "title" | "target_rub" | "started_at" | "manual_adjust_rub") {
                has_null = true;
            }
            continue;
        }
        match key.as_str() {
            "title" => patch.title = Some(as_str(v, key, 1, 200)?),
            "target_rub" => patch.target_rub = Some(as_positive(v, key)?),
            "started_at" => patch.started_at = Some(as_dt(v, key)?),
            "ends_at" => patch.ends_at = Some(Some(as_dt(v, key)?)),
            "manual_adjust_rub" => patch.manual_adjust_rub = Some(as_num(v, key)?),
            _ => {} // unknown fields are ignored, as in the Python version
        }
    }
    let goal = require_goal(&st)?;
    if has_null {
        return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, "поле не может быть пустым"));
    }
    st.store.update_goal(goal.id, &patch)?;
    if let Some(started) = patch.started_at {
        st.collectors.backfill(started);
    }
    st.hub.broadcast();
    Ok(Json(st.hub.snapshot()?))
}

async fn close_goal(State(st): State<AppState>) -> ApiResult {
    let goal = require_goal(&st)?;
    st.store.close_goal(goal.id, Utc::now())?;
    st.hub.broadcast();
    Ok(Json(st.hub.snapshot()?))
}

async fn history(State(st): State<AppState>) -> ApiResult {
    let items: Vec<Value> = st
        .store
        .history()?
        .into_iter()
        .map(|(g, total)| {
            json!({"id": g.id, "title": g.title, "target_rub": g.target_rub,
                   "collected_rub": (total * 100.0).round() / 100.0,
                   "started_at": to_iso(&g.started_at), "closed_at": g.closed_at.map(|d| to_iso(&d))})
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

async fn debug_donation(State(st): State<AppState>, body: Bytes) -> ApiResult {
    let m = body_object(&body)?;
    let amount = as_positive(required(&m, "amount_rub")?, "amount_rub")?;
    if amount > 1_000_000.0 {
        return Err(invalid("amount_rub", "Input should be less than or equal to 1000000"));
    }
    st.hub.add_test_amount(amount);
    Ok(Json(st.hub.snapshot()?))
}

async fn set_dx(State(st): State<AppState>, body: Bytes) -> ApiResult {
    let m = body_object(&body)?;
    let token = as_str(required(&m, "token")?, "token", 1, usize::MAX)?;
    st.store.set_setting("dx", &json!({"token": token.trim()}))?;
    st.collectors.restart("dx");
    Ok(Json(st.collectors.platforms()))
}

async fn set_da(State(st): State<AppState>, body: Bytes) -> ApiResult {
    let m = body_object(&body)?;
    let client_id = as_str(required(&m, "client_id")?, "client_id", 1, usize::MAX)?.trim().to_string();
    let secret = as_str(required(&m, "client_secret")?, "client_secret", 1, usize::MAX)?.trim().to_string();
    let mut cfg = st.store.get_setting("da")?.and_then(|v| v.as_object().cloned()).unwrap_or_default();
    if cfg.get("client_id").and_then(Value::as_str) != Some(client_id.as_str()) {
        cfg = Map::new(); // different app: old tokens are useless
    }
    cfg.insert("client_id".into(), json!(client_id));
    cfg.insert("client_secret".into(), json!(secret));
    st.store.set_setting("da", &Value::Object(cfg))?;
    st.collectors.restart("da");
    Ok(Json(st.collectors.platforms()))
}

async fn update_info(State(st): State<AppState>) -> Json<Value> {
    match st.updater.get() {
        None => Json(json!({"supported": false})),
        Some(u) => {
            let info = u.info();
            Json(json!({"supported": true, "current": info.current, "available": info.available, "installing": info.installing}))
        }
    }
}

async fn update_install(State(st): State<AppState>) -> Result<Response, ApiError> {
    let Some(u) = st.updater.get() else {
        return Err(fail(StatusCode::NOT_FOUND, "обновление недоступно в этом режиме"));
    };
    u.install().map_err(|e| fail(StatusCode::CONFLICT, e))?;
    Ok((StatusCode::ACCEPTED, Json(json!({"ok": true}))).into_response())
}

async fn da_start(State(st): State<AppState>) -> Result<Response, ApiError> {
    let cfg = st.store.get_setting("da")?.unwrap_or(Value::Null);
    let Some(client_id) = cfg["client_id"].as_str().filter(|s| !s.is_empty()) else {
        return Err(fail(StatusCode::BAD_REQUEST, "сначала укажите client_id и client_secret"));
    };
    let state = uuid::Uuid::new_v4().simple().to_string();
    st.oauth_states.lock().unwrap().insert(state.clone());
    let url = da::authorize_url(client_id, &state, &da::redirect_uri(st.port));
    Ok((StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, url)]).into_response())
}

async fn da_callback(State(st): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Result<Response, ApiError> {
    let state = q.get("state").map(String::as_str).unwrap_or("");
    if !st.oauth_states.lock().unwrap().remove(state) {
        return Err(fail(StatusCode::BAD_REQUEST, "неверный state, начните вход заново"));
    }
    let code = q.get("code").map(String::as_str).unwrap_or("");
    let error = q.get("error").map(String::as_str).unwrap_or("");
    if !error.is_empty() || code.is_empty() {
        let why = if error.is_empty() { "нет кода" } else { error };
        return Err(fail(StatusCode::BAD_REQUEST, format!("DonationAlerts отказал: {why}")));
    }
    let cfg = st.store.get_setting("da")?.unwrap_or_else(|| json!({}));
    let site = st.collectors.endpoints().da_site.clone();
    let cfg = da::exchange_code(st.collectors.client(), &site, &cfg, code, &da::redirect_uri(st.port))
        .await
        .map_err(|e| fail(StatusCode::BAD_GATEWAY, format!("обмен кода не удался: {}", describe_error(&e))))?;
    st.store.set_setting("da", &cfg)?;
    st.collectors.restart("da");
    // This page opens in the system browser, not in the app window.
    Ok(Html(DA_DONE_PAGE).into_response())
}
