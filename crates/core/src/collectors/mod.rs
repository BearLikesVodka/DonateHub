pub mod base;
pub mod donationalerts;
pub mod donatex;
pub mod manager;
pub mod signalr;

/// JSON number, or a string holding one (platform APIs are not consistent about it).
pub(crate) fn as_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}
