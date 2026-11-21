import asyncio
import logging

from fastapi import WebSocket

from .models import to_iso
from .store import Store
from .style import DEFAULT_STYLE, merge_style

log = logging.getLogger(__name__)


class Hub:
    """Current widget state and fan-out of it to every connected overlay."""

    def __init__(self, store: Store, test_ttl: float = 15.0):
        self.store = store
        self.test_ttl = test_ttl
        self.clients: set[WebSocket] = set()
        self._tasks: set[asyncio.Task] = set()
        # Test donations are never stored: they show on the bar for test_ttl seconds, whatever the goal dates.
        self._test_amounts: list[float] = []

    def style(self) -> dict:
        return merge_style(DEFAULT_STYLE, self.store.get_setting("style") or {})

    def snapshot(self) -> dict:
        goal = self.store.active_goal()
        return {
            "goal": None if goal is None else {
                "id": goal.id,
                "title": goal.title,
                "target_rub": goal.target_rub,
                "started_at": to_iso(goal.started_at),
                "ends_at": to_iso(goal.ends_at) if goal.ends_at else None,
                "manual_adjust_rub": goal.manual_adjust_rub,
            },
            "collected_rub": round(self.store.collected(goal) + sum(self._test_amounts), 2) if goal else 0,
            "style": self.style(),
        }

    def register(self, ws: WebSocket) -> None:
        self.clients.add(ws)

    def unregister(self, ws: WebSocket) -> None:
        self.clients.discard(ws)

    async def broadcast(self) -> None:
        snap = self.snapshot()
        for ws in list(self.clients):
            try:
                await ws.send_json(snap)
            except Exception:
                self.unregister(ws)

    def notify(self) -> None:
        task = asyncio.get_running_loop().create_task(self.broadcast())
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)

    def add_test_amount(self, amount_rub: float) -> None:
        self._test_amounts.append(amount_rub)
        task = asyncio.get_running_loop().create_task(self._expire_test_amount(amount_rub))
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)

    async def _expire_test_amount(self, amount_rub: float) -> None:
        await asyncio.sleep(self.test_ttl)
        self._test_amounts.remove(amount_rub)
        await self.broadcast()
