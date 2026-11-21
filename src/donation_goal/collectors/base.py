import asyncio
import logging
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Literal

log = logging.getLogger(__name__)

State = Literal["off", "connecting", "ok", "error"]

MAX_BACKOFF = 60
STABLE_AFTER = 60  # seconds of healthy connection before backoff resets


@dataclass
class CollectorStatus:
    state: State = "off"
    error: str | None = None


async def run_forever(
    name: str,
    connect: Callable[[Callable[[], None]], Awaitable[None]],
    status: CollectorStatus,
    on_status: Callable[[], None],
    sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
) -> None:
    """Keep `connect` running; it calls the passed `mark_ok` once connected and returns/raises on disconnect."""
    backoff = 1
    while True:
        status.state, status.error = "connecting", None
        on_status()
        started = time.monotonic()

        def mark_ok() -> None:
            status.state, status.error = "ok", None
            on_status()

        try:
            await connect(mark_ok)
            raise ConnectionError("connection closed")
        except asyncio.CancelledError:
            raise
        except Exception as e:
            log.warning("%s collector: %s", name, e, exc_info=not isinstance(e, ConnectionError))
            status.state, status.error = "error", str(e) or type(e).__name__
            on_status()
        if time.monotonic() - started > STABLE_AFTER:
            backoff = 1
        await sleep(backoff)
        backoff = min(backoff * 2, MAX_BACKOFF)
