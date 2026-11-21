import asyncio

import httpx
import pytest

from donatehub.collectors.base import CollectorStatus, run_forever


async def test_run_forever_backoff_and_status():
    status = CollectorStatus()
    sleeps = []
    attempts = 0
    changes = []

    async def connect(mark_ok):
        nonlocal attempts
        attempts += 1
        raise RuntimeError(f"fail {attempts}")

    async def fake_sleep(s):
        sleeps.append(s)
        if len(sleeps) >= 8:
            raise asyncio.CancelledError

    with pytest.raises(asyncio.CancelledError):
        await run_forever("dx", connect, status, lambda: changes.append(status.state), sleep=fake_sleep)

    assert sleeps == [1, 2, 4, 8, 16, 32, 60, 60]
    assert status.state == "error" and status.error == "fail 8"
    assert "connecting" in changes and "error" in changes


async def test_error_text_hides_token_and_is_readable():
    status = CollectorStatus()

    async def connect(mark_ok):
        req = httpx.Request("POST", "https://donatex.gg/api/hub/negotiate?negotiateVersion=1&access_token=SECRET")
        raise httpx.HTTPStatusError("401 for url ...access_token=SECRET", request=req, response=httpx.Response(401, request=req))

    async def stop(s):
        raise asyncio.CancelledError

    with pytest.raises(asyncio.CancelledError):
        await run_forever("dx", connect, status, lambda: None, sleep=stop)

    assert "SECRET" not in status.error
    assert "401" in status.error and "токен" in status.error.lower()
