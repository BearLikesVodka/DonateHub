//! Copy text to the system clipboard from the tray.

pub fn copy(text: String) {
    std::thread::spawn(move || {
        // On Wayland the copying process serves the clipboard; this thread lives until something else is copied.
        #[cfg(target_os = "linux")]
        let result = {
            use arboard::SetExtLinux;
            arboard::Clipboard::new().and_then(|mut c| c.set().wait().text(text))
        };
        #[cfg(windows)]
        let result = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
        if let Err(e) = result {
            tracing::warn!("clipboard: {e}");
        }
    });
}
