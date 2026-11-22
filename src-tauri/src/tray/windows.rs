//! Windows tray: Tauri's tray icon. Left click shows or hides the window, right click opens the menu.
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use super::{Action, MENU, overlay_url, run_action};
use crate::desktop::{APP_NAME, toggle_window};

/// Start the tray; returns whether the icon was created.
pub fn spawn(app: &AppHandle, port: u16) -> bool {
    match build(app, port) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("no system tray, closing the window will minimize it: {e}");
            false
        }
    }
}

fn build(app: &AppHandle, port: u16) -> tauri::Result<()> {
    let menu = Menu::new(app)?;
    for (i, (label, action)) in MENU.iter().enumerate() {
        if *action == Action::Quit {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        menu.append(&MenuItem::with_id(app, i.to_string(), *label, true, None::<&str>)?)?;
    }
    let url = overlay_url(port);
    TrayIconBuilder::with_id("donatehub")
        .icon(tauri::image::Image::from_bytes(include_bytes!("../../icons/32x32.png"))?)
        .tooltip(APP_NAME)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            let action = event.id().as_ref().parse::<usize>().ok().and_then(|i| MENU.get(i)).map(|(_, a)| *a);
            if let Some(action) = action {
                run_action(app, &url, action);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
