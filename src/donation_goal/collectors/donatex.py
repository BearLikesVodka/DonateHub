import logging
from collections.abc import Callable
from datetime import datetime

import httpx

from ..models import Donation, from_iso
from ..store import Store
from .signalr import signalr_stream

log = logging.getLogger(__name__)

API = "https://donatex.gg/api"
HUB_URL = "https://donatex.gg/api/public-donations-hub"
PAGE = 100
MAX_PAGES = 50


def parse_dx_donation(obj: dict) -> Donation:
    return Donation(
        platform="dx",
        external_id=str(obj["id"]),
        username=obj.get("username") or "",
        amount=float(obj.get("amount") or 0),
        currency=obj.get("currency") or "RUB",
        amount_rub=float(obj["amountInRub"]) if obj.get("amountInRub") is not None else None,
        created_at=from_iso(obj["timestamp"].replace("Z", "+00:00")),
        is_test=bool(obj.get("isTest")),
        raw=obj,
    )


class DonateXCollector:
    def __init__(self, store: Store, token: str, on_new: Callable[[], None], client: httpx.AsyncClient,
                 api: str = API, hub_url: str = HUB_URL):
        self.store = store
        self.token = token
        self.on_new = on_new
        self.client = client
        self.api = api
        self.hub_url = hub_url

    def _save(self, obj: dict) -> bool:
        try:
            d = parse_dx_donation(obj)
        except (KeyError, TypeError, ValueError) as e:
            log.warning("DonateX: unparseable donation %r: %s", obj, e)
            return False
        return self.store.add_donation(d)

    def handle_event(self, target: str, args: list) -> None:
        if target in ("DonationCreated", "DonationRerun") and args and isinstance(args[0], dict):
            if self._save(args[0]):
                self.on_new()

    def catch_up_since(self) -> datetime | None:
        latest = self.store.latest_donation_time("dx")
        if latest is not None:
            return latest
        goal = self.store.active_goal()
        return goal.started_at if goal else None

    async def catch_up(self, since: datetime) -> int:
        added = 0
        for page in range(MAX_PAGES):
            resp = await self.client.get(
                f"{self.api}/v1/donations",
                params={"skip": page * PAGE, "take": PAGE, "sortOrder": "NewestFirst"},
                headers={"Authorization": f"Bearer {self.token}"},
                timeout=15,
            )
            resp.raise_for_status()
            items = resp.json()
            reached_boundary = False
            for obj in items:
                try:
                    if from_iso(obj["timestamp"].replace("Z", "+00:00")) < since:
                        reached_boundary = True
                        break
                except (KeyError, TypeError, ValueError):
                    continue
                added += self._save(obj)
            if reached_boundary or len(items) < PAGE:
                break
        if added:
            self.on_new()
        return added

    async def connect(self, mark_ok: Callable[[], None]) -> None:
        async def opened() -> None:
            mark_ok()
            # Catch up after the socket is open so a donation can't slip between the REST read and the stream.
            since = self.catch_up_since()
            if since is not None:
                await self.catch_up(since)

        async for target, args in signalr_stream(self.hub_url, self.token, self.client, on_open=opened):
            self.handle_event(target, args)
