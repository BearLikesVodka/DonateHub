//! Tray icon: the menu and its actions are shared; Linux uses StatusNotifierItem (ksni) because Tauri's own tray
//! gets no clicks there, Windows uses Tauri's tray.
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::AppHandle;

use crate::desktop::{request_quit, show_window};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::spawn;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::spawn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Show,
    CopyOverlayUrl,
    Quit,
}

pub const MENU: [(&str, Action); 3] = [
    ("Открыть настройки", Action::Show),
    ("Скопировать ссылку для OBS", Action::CopyOverlayUrl),
    ("Выход", Action::Quit),
];

static TOLD_ABOUT_TRAY: AtomicBool = AtomicBool::new(false);

pub fn overlay_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/overlay")
}

pub fn run_action(app: &AppHandle, overlay_url: &str, action: Action) {
    match action {
        Action::Show => show_window(app),
        Action::CopyOverlayUrl => {
            crate::clipboard::copy(overlay_url.to_string());
            crate::notify::show(app, format!("Ссылка скопирована: {overlay_url}"));
        }
        Action::Quit => request_quit(app),
    }
}

pub fn on_hidden_to_tray(app: &AppHandle) {
    if !TOLD_ABOUT_TRAY.swap(true, Ordering::SeqCst) {
        crate::notify::show(app, "Приложение работает в трее. Оверлей в OBS продолжает обновляться.".into());
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, MENU};

    #[test]
    fn menu_labels() {
        let labels: Vec<_> = MENU.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels, ["Открыть настройки", "Скопировать ссылку для OBS", "Выход"]);
        assert_eq!(MENU.map(|(_, a)| a), [Action::Show, Action::CopyOverlayUrl, Action::Quit]);
    }
}
