//! Linux tray over StatusNotifierItem (KDE's native tray protocol): Tauri's own tray gets no clicks on Linux.
use tauri::{AppHandle, Manager};

use super::{Action, MENU, overlay_url, run_action};
use crate::desktop::{APP_NAME, toggle_window};

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
                    activate: Box::new(move |t: &mut Self| run_action(&t.app, &t.overlay_url, action)),
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
        &include_bytes!("../../icons/32x32.png")[..],
        &include_bytes!("../../icons/128x128.png")[..],
        &include_bytes!("../../icons/256x256.png")[..],
    ]
    .into_iter()
    .filter_map(icon)
    .collect();
    let tray = Tray { app: app.clone(), overlay_url: overlay_url(port), icons };
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
