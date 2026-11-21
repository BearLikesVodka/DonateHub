import asyncio
import logging

from fastapi import WebSocket

from .models import to_iso
from .store import Store
from .style import DEFAULT_STYLE, merge_style

log = logging.getLogger(__name__)


class Hub:
    """Current widget state and fan-out of it to every connected overlay."""

    def __init__(self, store: Store):
        self.store = store
        self.clients: set[WebSocket] = set()
        self._tasks: set[asyncio.Task] = set()

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
            "collected_rub": round(self.store.collected(goal), 2) if goal else 0,
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
