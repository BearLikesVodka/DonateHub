import threading
import time

import uvicorn
from fastapi import FastAPI


class ServerThread:
    """Runs uvicorn in a background thread so the Qt event loop can own the main thread."""

    def __init__(self, app: FastAPI, port: int):
        config = uvicorn.Config(app, host="127.0.0.1", port=port, log_level="warning", lifespan="on")
        # uvicorn skips signal handlers outside the main thread; Qt owns quitting.
        self.server = uvicorn.Server(config)
        self.thread = threading.Thread(target=self.server.run, name="uvicorn", daemon=True)

    def start(self) -> None:
        self.thread.start()

    def wait_ready(self, timeout: float = 10) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.server.started:
                return True
            if not self.thread.is_alive():
                return False
            time.sleep(0.05)
        return False

    def stop(self, timeout: float = 10) -> None:
        self.server.should_exit = True
        self.thread.join(timeout)
