use std::future::Future;
use std::ops::ControlFlow;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Serialize;

pub const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Healthy connection time after which the backoff resets.
pub const STABLE_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Off,
    Connecting,
    Ok,
    Error,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub state: State,
    pub error: Option<String>,
}

pub type SharedStatus = Arc<Mutex<Status>>;
pub type MarkOk = Arc<dyn Fn() + Send + Sync>;

static TOKEN_IN_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)((?:access_)?token=)[^&\s'"]+"#).unwrap());

pub fn redact(text: &str) -> String {
    TOKEN_IN_URL.replace_all(text, "${1}***").into_owned()
}

/// Short, token-free text for the status shown on the settings page.
pub fn describe_error(e: &anyhow::Error) -> String {
    if let Some(re) = e.chain().find_map(|c| c.downcast_ref::<reqwest::Error>()) {
        if let Some(code) = re.status() {
            return match code.as_u16() {
                401 => "Неверный или отозванный токен (HTTP 401)".into(),
                403 => "Токену не хватает прав (HTTP 403)".into(),
                429 => "Слишком много запросов, ждём (HTTP 429)".into(),
                c => format!("Сервер платформы ответил ошибкой HTTP {c}"),
            };
        }
        let kind = if re.is_timeout() {
            "TimeoutError"
        } else if re.is_connect() {
            "ConnectError"
        } else if re.is_decode() || re.is_body() {
            "ReadError"
        } else {
            "RequestError"
        };
        return format!("Нет связи с платформой: {kind}");
    }
    if e.chain().any(|c| c.is::<tokio_tungstenite::tungstenite::Error>()) {
        return "Нет связи с платформой: WebSocketError".into();
    }
    let text = redact(&format!("{e:#}"));
    if text.is_empty() { "Error".into() } else { text }
}

fn set(status: &SharedStatus, state: State, error: Option<String>) {
    let mut s = status.lock().unwrap();
    s.state = state;
    s.error = error;
}

/// Keep `connect` running; it calls `mark_ok` once connected and returns (Ok or Err) on disconnect.
/// Returns only when `sleep` says Break (tests); in the app the task is aborted instead.
pub async fn run_forever<C, Fut, S, SFut>(name: &str, mut connect: C, status: SharedStatus, mut sleep: S)
where
    C: FnMut(MarkOk) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
    S: FnMut(Duration) -> SFut,
    SFut: Future<Output = ControlFlow<()>>,
{
    let mut backoff = Duration::from_secs(1);
    loop {
        set(&status, State::Connecting, None);
        let started = Instant::now();
        let st = status.clone();
        let mark_ok: MarkOk = Arc::new(move || set(&st, State::Ok, None));
        let err = match connect(mark_ok).await {
            Ok(()) => anyhow::anyhow!("connection closed"),
            Err(e) => e,
        };
        let text = describe_error(&err);
        tracing::warn!("{name} collector: {text}");
        set(&status, State::Error, Some(text));
        if started.elapsed() > STABLE_AFTER {
            backoff = Duration::from_secs(1);
        }
        if sleep(backoff).await.is_break() {
            return;
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Production sleep for `run_forever`.
pub async fn real_sleep(d: Duration) -> ControlFlow<()> {
    tokio::time::sleep(d).await;
    ControlFlow::Continue(())
}
