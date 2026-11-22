//! Desktop notifications: notify-rust (freedesktop) on Linux, Windows toasts through the Tauri notification plugin.
use tauri::AppHandle;

use crate::desktop::APP_NAME;

#[cfg(target_os = "linux")]
pub fn show(_app: &AppHandle, body: String) {
    std::thread::spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname(APP_NAME).summary(APP_NAME).body(&body).show() {
            tracing::warn!("notification failed: {e}");
        }
    });
}

#[cfg(windows)]
pub fn show(app: &AppHandle, body: String) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(APP_NAME).body(body).show() {
        tracing::warn!("notification failed: {e}");
    }
}
