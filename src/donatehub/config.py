import os
import socket
from collections.abc import Mapping
from pathlib import Path

from .store import Store

DEFAULT_PORT = 47321
APP_DIR = "donatehub"
OLD_APP_DIR = "donation-goal"
PORT_ENV = "DONATEHUB_PORT"


def data_dir(env: Mapping[str, str] = os.environ) -> Path:
    base = env.get("XDG_DATA_HOME") or str(Path(env.get("HOME") or Path.home()) / ".local" / "share")
    path = Path(base) / APP_DIR
    old = Path(base) / OLD_APP_DIR
    if not path.exists() and old.is_dir():
        old.rename(path)  # keep goals, donations and tokens from before the DonateHub rename
    path.mkdir(parents=True, exist_ok=True)
    return path


def valid_port(value: object) -> int | None:
    try:
        port = int(value)  # type: ignore[arg-type]
    except (TypeError, ValueError):
        return None
    return port if 1024 <= port <= 65535 else None


def resolve_port(store: Store, env: Mapping[str, str] = os.environ) -> int:
    return valid_port(env.get(PORT_ENV)) or valid_port(store.get_setting("port")) or DEFAULT_PORT


def port_free(port: int) -> bool:
    with socket.socket() as s:
        # Same option uvicorn uses, so lingering TIME_WAIT connections don't count as "busy".
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            s.bind(("127.0.0.1", port))
        except OSError:
            return False
    return True
