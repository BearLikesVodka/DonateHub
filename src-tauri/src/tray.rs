//! Tray icon over StatusNotifierItem (KDE's native tray protocol): Tauri's own tray gets no clicks on Linux.
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager};

use crate::desktop::{APP_NAME, request_quit, show_window, toggle_window};

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

struct Tray {
    app: AppHandle,
    overlay_url: String,
    icons: Vec<ksni::Icon>,
}

/// Keeps the tray service alive for the app's lifetime.
struct TrayHandle(#[allow(dead_code)] ksni::Handle<Tray>);

fn icon(png: &[u8]) -> Option<ksni::Icon> {
    let img = tauri::image::Image::from_bytes(png).ok()?;
    // RGBA -> ARGB32 in network byte order, as StatusNotifierItem wants.
    let data = img.rgba().as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [a, r, g, b]).collect();
    Some(ksni::Icon { width: img.width() as i32, height: img.height() as i32, data })
}

fn notify(body: String) {
    std::thread::spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname(APP_NAME).summary(APP_NAME).body(&body).show() {
            tracing::warn!("notification failed: {e}");
        }
    });
}

fn copy_to_clipboard(text: String) {
    // On Wayland the copying process serves the clipboard; this thread lives until something else is copied.
    std::thread::spawn(move || {
        use arboard::SetExtLinux;
        let result = arboard::Clipboard::new().and_then(|mut c| c.set().wait().text(text));
        if let Err(e) = result {
            tracing::warn!("clipboard: {e}");
        }
    });
}

impl Tray {
    fn run(&self, action: Action) {
        match action {
            Action::Show => show_window(&self.app),
            Action::CopyOverlayUrl => {
                copy_to_clipboard(self.overlay_url.clone());
                notify(format!("Ссылка скопирована: {}", self.overlay_url));
            }
            Action::Quit => request_quit(&self.app),
        }
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "donatehub".into()
    }

    fn title(&self) -> String {
        APP_NAME.into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: APP_NAME.into(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        toggle_window(&self.app);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut items = Vec::new();
        for (label, action) in MENU {
            if action == Action::Quit {
                items.push(ksni::MenuItem::Separator);
            }
            items.push(
                ksni::menu::StandardItem {
                    label: label.into(),
                    activate: Box::new(move |t: &mut Self| t.run(action)),
                    ..Default::default()
                }
                .into(),
            );
        }
        items
    }
}

/// Start the tray; returns whether a tray host (StatusNotifierWatcher) accepted it.
pub fn spawn(app: &AppHandle, port: u16) -> bool {
    use ksni::TrayMethods;
    let icons = [
        &include_bytes!("../icons/32x32.png")[..],
        &include_bytes!("../icons/128x128.png")[..],
        &include_bytes!("../icons/256x256.png")[..],
    ]
    .into_iter()
    .filter_map(icon)
    .collect();
    let tray = Tray { app: app.clone(), overlay_url: format!("http://127.0.0.1:{port}/overlay"), icons };
    match tauri::async_runtime::block_on(tray.spawn()) {
        Ok(handle) => {
            app.manage(TrayHandle(handle));
            true
        }
        Err(e) => {
            tracing::warn!("no system tray, closing the window will minimize it: {e}");
            false
        }
    }
}

pub fn on_hidden_to_tray(_app: &AppHandle) {
    if !TOLD_ABOUT_TRAY.swap(true, Ordering::SeqCst) {
        notify("Приложение работает в трее. Оверлей в OBS продолжает обновляться.".into());
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
