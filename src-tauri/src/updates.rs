//! Auto-update from GitHub releases: checks in the background, installs only when the user presses «Обновить».
use std::sync::{Arc, Mutex};
use std::time::Duration;

use donatehub_core::updates::{UpdateInfo, Updater};
use tauri::{AppHandle, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Overrides the `latest.json` URL, e.g. a local server for testing a draft release. Signatures are still checked.
const ENDPOINT_ENV: &str = "DONATEHUB_UPDATE_ENDPOINT";
const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// What the updater knows, kept apart from Tauri so the rules are unit-testable.
#[derive(Debug, Default)]
pub struct UpdateState {
    available: Option<String>,
    notified: Option<String>,
    installing: bool,
}

impl UpdateState {
    /// Remember the newest version found; true when the user has not been told about this version yet.
    pub fn set_available(&mut self, version: Option<String>) -> bool {
        self.available = version;
        match &self.available {
            Some(v) if self.notified.as_ref() != Some(v) => {
                self.notified = Some(v.clone());
                true
            }
            _ => false,
        }
    }

    pub fn begin_install(&mut self) -> Result<(), String> {
        if self.available.is_none() {
            return Err("обновлений нет".into());
        }
        if self.installing {
            return Err("обновление уже устанавливается".into());
        }
        self.installing = true;
        Ok(())
    }

    pub fn install_failed(&mut self) {
        self.installing = false;
    }

    pub fn available(&self) -> Option<String> {
        self.available.clone()
    }

    pub fn installing(&self) -> bool {
        self.installing
    }
}

/// The updater can replace the NSIS install on Windows and the AppImage file on Linux; a bare binary
/// (`cargo run`, a distro package) cannot update itself.
pub fn supported(is_windows: bool, appimage_env: Option<&str>) -> bool {
    is_windows || appimage_env.is_some_and(|p| !p.is_empty())
}

struct Inner {
    app: AppHandle,
    state: Mutex<UpdateState>,
    update: Mutex<Option<Update>>,
}

/// The shell's updater, registered in the core so the settings page can show and start updates.
#[derive(Clone)]
pub struct DesktopUpdater(Arc<Inner>);

impl DesktopUpdater {
    /// The updater for this run, or None when this build cannot replace itself.
    pub fn new(app: &AppHandle) -> Option<DesktopUpdater> {
        let appimage = std::env::var("APPIMAGE").ok();
        if !supported(cfg!(windows), appimage.as_deref()) {
            return None;
        }
        Some(DesktopUpdater(Arc::new(Inner { app: app.clone(), state: Mutex::default(), update: Mutex::default() })))
    }

    /// Check shortly after start and then every 6 hours, for the app's lifetime.
    pub fn spawn_checks(&self) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK).await;
            loop {
                this.check().await;
                tokio::time::sleep(CHECK_EVERY).await;
            }
        });
    }

    fn updater(&self) -> tauri_plugin_updater::Result<tauri_plugin_updater::Updater> {
        let mut builder = self.0.app.updater_builder();
        if let Some(url) = std::env::var(ENDPOINT_ENV).ok().and_then(|u| Url::parse(&u).ok()) {
            builder = builder.endpoints(vec![url])?;
        }
        let app = self.0.app.clone();
        // Windows runs the installer and calls process::exit: free the single-instance lock so the new version starts.
        builder.on_before_exit(move || tauri_plugin_single_instance::destroy(&app)).build()
    }

    async fn check(&self) {
        if self.0.state.lock().unwrap().installing() {
            return;
        }
        let found = match self.updater() {
            Ok(u) => u.check().await,
            Err(e) => Err(e),
        };
        let update = match found {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("update check failed: {e}");
                return;
            }
        };
        let version = update.as_ref().map(|u| u.version.clone());
        *self.0.update.lock().unwrap() = update;
        let tell = self.0.state.lock().unwrap().set_available(version.clone());
        if let (true, Some(v)) = (tell, version) {
            crate::notify::show(&self.0.app, format!("Доступна версия {v}. Обновить можно в окне настроек."));
        }
    }

    async fn download_and_install(self) {
        let update = self.0.update.lock().unwrap().clone();
        let Some(update) = update else {
            self.0.state.lock().unwrap().install_failed();
            return;
        };
        let bytes = match update.download(|_, _| {}, || {}).await {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("update download failed: {e}");
                self.0.state.lock().unwrap().install_failed();
                return;
            }
        };
        // Only with the package downloaded: stop collectors and free the port for the new version.
        crate::desktop::stop_server(&self.0.app).await;
        if let Err(e) = update.install(bytes) {
            tracing::error!("update install failed: {e}");
        }
        // Linux: the AppImage file was replaced, start it. Windows does not get here after a successful install;
        // after a failed one the restart brings the server back.
        self.0.app.request_restart();
    }
}

impl Updater for DesktopUpdater {
    fn info(&self) -> UpdateInfo {
        let s = self.0.state.lock().unwrap();
        UpdateInfo {
            current: self.0.app.package_info().version.to_string(),
            available: s.available(),
            installing: s.installing(),
        }
    }

    fn install(&self) -> Result<(), String> {
        self.0.state.lock().unwrap().begin_install()?;
        tauri::async_runtime::spawn(self.clone().download_and_install());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{UpdateState, supported};

    #[test]
    fn notifies_once_per_version() {
        let mut s = UpdateState::default();
        assert!(s.set_available(Some("0.3.1".into())));
        assert!(!s.set_available(Some("0.3.1".into())));
        assert!(!s.set_available(None));
        assert!(!s.set_available(Some("0.3.1".into())));
        assert!(s.set_available(Some("0.3.2".into())));
    }

    #[test]
    fn install_requires_available_update() {
        let mut s = UpdateState::default();
        assert_eq!(s.begin_install(), Err("обновлений нет".to_string()));
        s.set_available(None);
        assert_eq!(s.begin_install(), Err("обновлений нет".to_string()));
    }

    #[test]
    fn install_not_started_twice() {
        let mut s = UpdateState::default();
        s.set_available(Some("0.3.1".into()));
        assert_eq!(s.begin_install(), Ok(()));
        assert_eq!(s.begin_install(), Err("обновление уже устанавливается".to_string()));
        s.install_failed();
        assert_eq!(s.begin_install(), Ok(()));
    }

    #[test]
    fn supported_only_on_windows_or_appimage() {
        assert!(supported(true, None));
        assert!(supported(false, Some("/home/u/Applications/DonateHub.AppImage")));
        assert!(!supported(false, None));
        assert!(!supported(false, Some("")));
    }
}
