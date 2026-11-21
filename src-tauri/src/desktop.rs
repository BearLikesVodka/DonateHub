//! Desktop shell: a Tauri window on the local settings page; closing it hides to the tray, the server keeps running.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use donatehub_core::app::build;
use donatehub_core::config::port_free;
use donatehub_core::server::Server;
use donatehub_core::store::Store;
use serde_json::json;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

pub const APP_NAME: &str = "DonateHub";
const MAIN: &str = "main";
const LOCAL_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];
/// Pages that must open in the system browser even though they are served locally.
const EXTERNAL_LOCAL_PATHS: [&str; 1] = ["/auth/da/start"];

/// State shared by the window, the tray and signal handlers.
#[derive(Default)]
pub struct Shell {
    server: Mutex<Option<Server>>,
    quitting: AtomicBool,
    pub tray_available: AtomicBool,
}

pub fn is_internal(url: &Url) -> bool {
    if matches!(url.scheme(), "about" | "data" | "blob") {
        return true;
    }
    url.host_str().is_some_and(|h| LOCAL_HOSTS.contains(&h)) && !EXTERNAL_LOCAL_PATHS.contains(&url.path())
}

fn open_external(app: &AppHandle, url: &Url) {
    if let Err(e) = app.opener().open_url(url.as_str(), None::<&str>) {
        tracing::warn!("cannot open {url} in the browser: {e}");
    }
}

pub fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[expect(dead_code, reason = "used by the tray")]
pub fn toggle_window(app: &AppHandle) {
    let Some(w) = app.get_webview_window(MAIN) else { return };
    let shown = w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
    if shown {
        let _ = w.hide();
    } else {
        show_window(app);
    }
}

/// Really quit: stop the server (collectors, overlay sockets), then exit the event loop.
pub fn request_quit(app: &AppHandle) {
    let shell = app.state::<Shell>();
    if shell.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    let server = shell.server.lock().unwrap().take();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(server) = server {
            server.stop().await;
        }
        app.exit(0);
    });
}

fn suggest_port(port: u16) -> u16 {
    (port.saturating_add(1)..=65535).chain(47321..port).find(|p| *p >= 1024 && port_free(*p)).unwrap_or(47321)
}

fn ask_new_port(app: &AppHandle, store: Store, port: u16) {
    let new_port = suggest_port(port);
    let app2 = app.clone();
    app.dialog()
        .message(format!(
            "Порт {port} занят другой программой.\nИспользовать порт {new_port}? После перезапуска обновите ссылку \
             в OBS и Redirect URL в приложении DonationAlerts."
        ))
        .title(APP_NAME)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(format!("Использовать {new_port}"), "Выход".into()))
        .show(move |ok| {
            if !ok {
                app2.exit(1);
                return;
            }
            if let Err(e) = store.set_setting("port", &json!(new_port)) {
                tracing::error!("port not saved: {e:#}");
            }
            let app3 = app2.clone();
            app2.dialog()
                .message(format!("Порт {new_port} сохранён. Запустите приложение снова."))
                .title(APP_NAME)
                .show(move |_| app3.exit(1));
        });
}

fn fatal(app: &AppHandle, text: String) {
    let app2 = app.clone();
    app.dialog().message(text).title(APP_NAME).kind(MessageDialogKind::Error).show(move |_| app2.exit(1));
}

fn create_window(app: &AppHandle, port: u16, visible: bool) -> tauri::Result<()> {
    let url: Url = format!("http://127.0.0.1:{port}/").parse().expect("valid url");
    let nav_app = app.clone();
    let popup_app = app.clone();
    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::External(url))
        .title(APP_NAME)
        .inner_size(1280.0, 860.0)
        .visible(visible)
        .on_navigation(move |url| {
            if is_internal(url) {
                return true;
            }
            // DonationAlerts login and outside links go to the system browser, where the streamer is signed in.
            open_external(&nav_app, url);
            false
        })
        .on_new_window(move |url, _| {
            open_external(&popup_app, &url);
            NewWindowResponse::Deny
        })
        .build()?;
    // Same as the Qt version's JavascriptCanAccessClipboard: the "Копировать" buttons write to the clipboard.
    window.with_webview(|wv| {
        use webkit2gtk::{SettingsExt, WebViewExt};
        if let Some(settings) = wv.inner().settings() {
            settings.set_javascript_can_access_clipboard(true);
        }
    })?;
    Ok(())
}

fn on_close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    let app = window.app_handle();
    let shell = app.state::<Shell>();
    if shell.quitting.load(Ordering::SeqCst) {
        return;
    }
    // Closing must never stop the server: OBS keeps reading the overlay.
    api.prevent_close();
    if shell.tray_available.load(Ordering::SeqCst) {
        let _ = window.hide();
        crate::tray::on_hidden_to_tray(app);
    } else {
        let _ = window.minimize();
    }
}

fn quit_on_signals(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        request_quit(&app);
    });
}

pub fn run(store: Store, port: u16, hidden: bool) -> anyhow::Result<i32> {
    let app = tauri::Builder::default()
        // Must be first: a second launch only shows the running window, then exits with 0.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_window(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Shell::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            if !port_free(port) {
                ask_new_port(&handle, store, port);
                return Ok(());
            }
            let started = tauri::async_runtime::block_on(async {
                let (router, state) = build(store, true, port);
                Server::start(router, state.collectors.clone(), port).await
            });
            let server = match started {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("server start failed: {e:#}");
                    fatal(&handle, format!("Не удалось запустить сервер на порту {port}."));
                    return Ok(());
                }
            };
            *app.state::<Shell>().server.lock().unwrap() = Some(server);
            let tray = crate::tray::spawn(&handle, port);
            app.state::<Shell>().tray_available.store(tray, Ordering::SeqCst);
            create_window(&handle, port, !hidden || !tray)?;
            quit_on_signals(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                on_close_requested(window, api);
            }
        })
        .build(tauri::generate_context!())?;
    app.run(|_, event| {
        // A hidden window is not "closed"; still, never let Tauri exit on its own — only request_quit exits.
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::is_internal;
    use tauri::Url;

    #[test]
    fn is_internal_rules() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(is_internal(&u("http://127.0.0.1:47321/")));
        assert!(is_internal(&u("http://localhost:47321/overlay")));
        assert!(is_internal(&u("about:blank")));
        assert!(!is_internal(&u("http://127.0.0.1:47321/auth/da/start")));
        assert!(!is_internal(&u("https://www.donationalerts.com/application/clients")));
        assert!(!is_internal(&u("http://127.0.0.1.evil.example/")));
    }
}
