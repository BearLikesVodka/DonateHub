use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use donatehub_core::collectors::base::{State, Status, run_forever};

#[tokio::test]
async fn test_run_forever_backoff_and_status() {
    let status = Arc::new(Mutex::new(Status::default()));
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let seen_states = Arc::new(Mutex::new(Vec::new()));
    let mut attempts = 0;
    let (st, seen) = (status.clone(), seen_states.clone());
    run_forever(
        "dx",
        move |_mark_ok| {
            attempts += 1;
            seen.lock().unwrap().push(st.lock().unwrap().state);
            let n = attempts;
            async move { Err(anyhow::anyhow!("fail {n}")) }
        },
        status.clone(),
        {
            let sleeps = sleeps.clone();
            move |d: Duration| {
                sleeps.lock().unwrap().push(d.as_secs());
                let stop = sleeps.lock().unwrap().len() >= 8;
                async move { if stop { ControlFlow::Break(()) } else { ControlFlow::Continue(()) } }
            }
        },
    )
    .await;
    assert_eq!(*sleeps.lock().unwrap(), [1, 2, 4, 8, 16, 32, 60, 60]);
    let s = status.lock().unwrap().clone();
    assert_eq!(s.state, State::Error);
    assert_eq!(s.error.as_deref(), Some("fail 8"));
    assert!(seen_states.lock().unwrap().iter().all(|s| *s == State::Connecting));
}

#[tokio::test]
async fn test_error_text_hides_token_and_is_readable() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let url = format!("{}/api/hub/negotiate?negotiateVersion=1&access_token=SECRET", server.uri());
    let status = Arc::new(Mutex::new(Status::default()));
    run_forever(
        "dx",
        move |_| {
            let url = url.clone();
            async move {
                reqwest::Client::new().post(&url).send().await?.error_for_status()?;
                Ok(())
            }
        },
        status.clone(),
        |_| async { ControlFlow::Break(()) },
    )
    .await;
    let err = status.lock().unwrap().error.clone().unwrap();
    assert!(!err.contains("SECRET"));
    assert!(err.contains("401") && err.to_lowercase().contains("токен"), "{err}");
}

#[test]
fn redact_hides_tokens_in_urls() {
    use donatehub_core::collectors::base::{describe_error, redact};
    assert_eq!(redact("wss://x/hub?id=1&access_token=abc&y=2"), "wss://x/hub?id=1&access_token=***&y=2");
    assert_eq!(redact("Token=abc def"), "Token=*** def");
    assert_eq!(describe_error(&anyhow::anyhow!("bad url ?token=zzz")), "bad url ?token=***");
}
