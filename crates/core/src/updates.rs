//! Hook for the desktop shell's auto-updater: the settings page reaches it through `/api/update`.
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub available: Option<String>,
    pub installing: bool,
    /// Why the last install attempt failed, until the next attempt.
    pub error: Option<String>,
}

pub trait Updater: Send + Sync + 'static {
    fn info(&self) -> UpdateInfo;
    /// Start installing the available update in the background. Err(text for the user) if it cannot start.
    fn install(&self) -> Result<(), String>;
}
