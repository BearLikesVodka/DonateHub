import socket

from donation_goal.config import DEFAULT_PORT, data_dir, port_free, resolve_port


def test_data_dir_uses_xdg(tmp_path):
    d = data_dir({"XDG_DATA_HOME": str(tmp_path / "xdg"), "HOME": str(tmp_path / "home")})
    assert d == tmp_path / "xdg" / "donation-goal"
    assert d.is_dir()


def test_data_dir_default_home(tmp_path):
    d = data_dir({"HOME": str(tmp_path)})
    assert d == tmp_path / ".local" / "share" / "donation-goal"
    assert d.is_dir()


def test_resolve_port_priority(store):
    assert DEFAULT_PORT == 47321
    assert resolve_port(store, {}) == 47321
    store.set_setting("port", 48000)
    assert resolve_port(store, {}) == 48000
    assert resolve_port(store, {"DONATION_GOAL_PORT": "49000"}) == 49000
    assert resolve_port(store, {"DONATION_GOAL_PORT": "nope"}) == 48000
    assert resolve_port(store, {"DONATION_GOAL_PORT": "80"}) == 48000
    store.set_setting("port", "garbage")
    assert resolve_port(store, {}) == 47321


def test_port_free_detects_busy_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    s.listen()
    port = s.getsockname()[1]
    assert port_free(port) is False
    s.close()
    assert port_free(port) is True
