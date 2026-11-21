import asyncio

import pytest

from donation_goal.collectors.base import CollectorStatus, run_forever


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
