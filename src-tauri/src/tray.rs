//! Tray icon (filled in by the tray task); for now there is no tray, so the window minimizes instead of hiding.
use tauri::AppHandle;

/// Start the tray; returns whether a tray host (StatusNotifierWatcher) is available.
pub fn spawn(_app: &AppHandle, _port: u16) -> bool {
    false
}

pub fn on_hidden_to_tray(_app: &AppHandle) {}
