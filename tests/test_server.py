import socket

import httpx

from donation_goal.app import create_app
from donation_goal.config import port_free
from donation_goal.server import ServerThread
from donation_goal.store import Store


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def test_server_thread_stop_releases_port():
    port = free_port()
    srv = ServerThread(create_app(Store(":memory:"), start_collectors=False, port=port), port)
    srv.start()
    assert srv.wait_ready(10) is True
    r = httpx.get(f"http://127.0.0.1:{port}/api/state", timeout=5)
    assert r.status_code == 200 and r.json()["port"]["current"] == port
    assert port_free(port) is False
    srv.stop()
    assert port_free(port) is True
