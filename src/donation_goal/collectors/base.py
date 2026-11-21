import asyncio
import logging
import re
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Literal

import httpx

log = logging.getLogger(__name__)

State = Literal["off", "connecting", "ok", "error"]

MAX_BACKOFF = 60
STABLE_AFTER = 60  # seconds of healthy connection before backoff resets


_TOKEN_IN_URL = re.compile(r"((?:access_)?token=)[^&\s'\"]+", re.IGNORECASE)


def redact(text: str) -> str:
    return _TOKEN_IN_URL.sub(r"\1***", text)


def describe_error(e: BaseException) -> str:
    """Short, token-free text for the status shown on the settings page."""
    if isinstance(e, httpx.HTTPStatusError):
        code = e.response.status_code
        if code == 401:
            return "Неверный или отозванный токен (HTTP 401)"
        if code == 403:
            return "Токену не хватает прав (HTTP 403)"
        if code == 429:
            return "Слишком много запросов, ждём (HTTP 429)"
        return f"Сервер платформы ответил ошибкой HTTP {code}"
    if isinstance(e, httpx.TransportError):
        return f"Нет связи с платформой: {type(e).__name__}"
    return redact(str(e)) or type(e).__name__


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
            text = describe_error(e)
            expected = isinstance(e, (ConnectionError, OSError, httpx.HTTPError, TimeoutError))
            log.warning("%s collector: %s", name, text, exc_info=not expected)
            status.state, status.error = "error", text
            on_status()
        if time.monotonic() - started > STABLE_AFTER:
            backoff = 1
        await sleep(backoff)
        backoff = min(backoff * 2, MAX_BACKOFF)
