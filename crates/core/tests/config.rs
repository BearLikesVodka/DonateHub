use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::path::Path;

use donatehub_core::config::{data_dir, port_free, windows_data_dir};

fn env(pairs: &[(&str, &Path)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> =
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string_lossy().into_owned())).collect();
    move |k| map.get(k).cloned()
}

#[cfg(not(windows))]
#[test]
fn test_data_dir_uses_xdg() {
    let tmp = tempfile::tempdir().unwrap();
    let d = data_dir(&env(&[("XDG_DATA_HOME", &tmp.path().join("xdg")), ("HOME", &tmp.path().join("home"))])).unwrap();
    assert_eq!(d, tmp.path().join("xdg").join("donatehub"));
    assert!(d.is_dir());
}

#[cfg(not(windows))]
#[test]
fn test_data_dir_default_home() {
    let tmp = tempfile::tempdir().unwrap();
    let d = data_dir(&env(&[("HOME", tmp.path())])).unwrap();
    assert_eq!(d, tmp.path().join(".local").join("share").join("donatehub"));
    assert!(d.is_dir());
}

#[cfg(not(windows))]
#[test]
fn test_data_dir_migrates_old_donation_goal_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("xdg").join("donation-goal");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("app.db"), b"live data").unwrap();
    let d = data_dir(&env(&[("XDG_DATA_HOME", &tmp.path().join("xdg"))])).unwrap();
    assert_eq!(d, tmp.path().join("xdg").join("donatehub"));
    assert_eq!(std::fs::read(d.join("app.db")).unwrap(), b"live data");
    assert!(!old.exists());
}

#[cfg(not(windows))]
#[test]
fn test_data_dir_keeps_existing_new_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("xdg").join("donation-goal");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("app.db"), b"old").unwrap();
    let new = tmp.path().join("xdg").join("donatehub");
    std::fs::create_dir(&new).unwrap();
    std::fs::write(new.join("app.db"), b"new").unwrap();
    let d = data_dir(&env(&[("XDG_DATA_HOME", &tmp.path().join("xdg"))])).unwrap();
    assert_eq!(std::fs::read(d.join("app.db")).unwrap(), b"new");
    assert_eq!(std::fs::read(old.join("app.db")).unwrap(), b"old");
}

#[test]
fn windows_data_dir_uses_appdata() {
    let tmp = tempfile::tempdir().unwrap();
    let d = windows_data_dir(&env(&[("APPDATA", &tmp.path().join("Roaming"))])).unwrap();
    assert_eq!(d, tmp.path().join("Roaming").join("DonateHub"));
    assert!(d.is_dir());
}

#[test]
fn windows_data_dir_requires_appdata() {
    assert!(windows_data_dir(&env(&[])).is_err());
}

#[cfg(windows)]
#[test]
fn data_dir_on_windows_uses_appdata() {
    let tmp = tempfile::tempdir().unwrap();
    let d = data_dir(&env(&[("APPDATA", tmp.path()), ("XDG_DATA_HOME", &tmp.path().join("xdg"))])).unwrap();
    assert_eq!(d, tmp.path().join("DonateHub"));
}

#[test]
fn test_port_free_detects_busy_port() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    assert!(!port_free(port));
    drop(l);
    assert!(port_free(port));
}

#[test]
fn test_port_free_ignores_time_wait() {
    // Right after the app quits, server-closed connections linger in TIME_WAIT; the server can still bind.
    let srv = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    srv.set_reuse_address(true).unwrap();
    srv.bind(&"127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap().into()).unwrap();
    srv.listen(1).unwrap();
    let port = srv.local_addr().unwrap().as_socket().unwrap().port();
    let cli = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let (conn, _) = srv.accept().unwrap();
    drop(conn); // server side closes first -> TIME_WAIT on the server port
    drop(srv);
    drop(cli);
    assert!(port_free(port));
}
