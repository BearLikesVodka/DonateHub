use std::net::SocketAddr;
use std::path::PathBuf;

use serde_json::Value;
use socket2::{Domain, Socket, Type};

pub const DEFAULT_PORT: u16 = 47321;
pub const APP_DIR: &str = "donatehub";
pub const OLD_APP_DIR: &str = "donation-goal";
pub const PORT_ENV: &str = "DONATEHUB_PORT";

/// `$XDG_DATA_HOME/donatehub` or `~/.local/share/donatehub`, created if missing.
pub fn data_dir(env: &impl Fn(&str) -> Option<String>) -> std::io::Result<PathBuf> {
    let base = match env("XDG_DATA_HOME").filter(|s| !s.is_empty()) {
        Some(x) => PathBuf::from(x),
        None => {
            let home = env("HOME").filter(|s| !s.is_empty()).map(PathBuf::from).or_else(std::env::home_dir);
            home.ok_or_else(|| std::io::Error::other("HOME is not set"))?.join(".local").join("share")
        }
    };
    let path = base.join(APP_DIR);
    let old = base.join(OLD_APP_DIR);
    if !path.exists() && old.is_dir() {
        std::fs::rename(&old, &path)?; // keep goals, donations and tokens from before the DonateHub rename
    }
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn data_dir_from_env() -> std::io::Result<PathBuf> {
    data_dir(&|k| std::env::var(k).ok())
}

pub fn valid_port_str(s: &str) -> Option<u16> {
    s.trim().parse::<i64>().ok().and_then(in_range)
}

/// Same leniency as Python's `int(value)`: an integer, or a string holding one.
pub fn valid_port_json(v: &Value) -> Option<u16> {
    match v {
        Value::Number(n) => n.as_i64().and_then(in_range),
        Value::String(s) => valid_port_str(s),
        _ => None,
    }
}

fn in_range(n: i64) -> Option<u16> {
    (1024..=65535).contains(&n).then_some(n as u16)
}

pub fn port_free(port: u16) -> bool {
    let Ok(s) = Socket::new(Domain::IPV4, Type::STREAM, None) else { return false };
    // Same option the server uses, so lingering TIME_WAIT connections don't count as "busy".
    if s.set_reuse_address(true).is_err() {
        return false;
    }
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    s.bind(&addr.into()).is_ok()
}
