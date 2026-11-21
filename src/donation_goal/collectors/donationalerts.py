import asyncio
import json
import logging
import time
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Protocol
from urllib.parse import urlencode

import httpx
from websockets.asyncio.client import connect

from ..models import Donation
from ..store import Store

log = logging.getLogger(__name__)

SITE = "https://www.donationalerts.com"
CENTRIFUGO_URL = "wss://centrifugo.donationalerts.com/connection/websocket"
REDIRECT_URI = "http://127.0.0.1:8000/auth/da/callback"
SCOPES = "oauth-user-show oauth-donation-subscribe oauth-donation-index"
SETTINGS_KEY = "da"
REFRESH_MARGIN = 86400
MAX_PAGES = 50
CENTRIFUGO_PING = 25


class Rates(Protocol):
    async def to_rub(self, amount: float, currency: str) -> float | None: ...


def authorize_url(client_id: str, state: str) -> str:
    q = urlencode({"client_id": client_id, "redirect_uri": REDIRECT_URI, "response_type": "code",
                   "scope": SCOPES, "state": state})
    return f"{SITE}/oauth/authorize?{q}"


def _apply_token(cfg: dict, resp: dict) -> dict:
    return {
        **cfg,
        "access_token": resp["access_token"],
        # DA may omit a new refresh token; the old one stays valid then.
        "refresh_token": resp.get("refresh_token") or cfg.get("refresh_token"),
        "expires_at": time.time() + float(resp.get("expires_in") or 3600),
    }


async def exchange_code(client: httpx.AsyncClient, cfg: dict, code: str) -> dict:
    resp = await client.post(f"{SITE}/oauth/token", data={
        "grant_type": "authorization_code", "client_id": cfg["client_id"],
        "client_secret": cfg["client_secret"], "redirect_uri": REDIRECT_URI, "code": code,
    }, timeout=15)
    resp.raise_for_status()
    return _apply_token(cfg, resp.json())


async def refresh(client: httpx.AsyncClient, cfg: dict) -> dict:
    resp = await client.post(f"{SITE}/oauth/token", data={
        "grant_type": "refresh_token", "refresh_token": cfg["refresh_token"],
        "client_id": cfg["client_id"], "client_secret": cfg["client_secret"], "scope": SCOPES,
    }, timeout=15)
    resp.raise_for_status()
    return _apply_token(cfg, resp.json())


def parse_created_at(s: str) -> datetime:
    # Docs say "YYYY-MM-DD HH.MM.SS", examples use colons; DA times are UTC.
    date, _, clock = s.strip().partition(" ")
    return datetime.fromisoformat(f"{date}T{clock.replace('.', ':')}").replace(tzinfo=UTC)


def parse_da_donation(obj: dict, amount_rub: float | None) -> Donation:
    return Donation(
        platform="da",
        external_id=str(obj["id"]),
        username=obj.get("username") or "",
        amount=float(obj["amount"]),
        currency=(obj.get("currency") or "RUB").upper(),
        amount_rub=amount_rub,
        created_at=parse_created_at(obj["created_at"]),
        is_test=False,
        raw=obj,
    )


def _is_donation(x: object) -> bool:
    return isinstance(x, dict) and "id" in x and "amount" in x


def extract_push_donation(msg: dict) -> dict | None:
    data = (msg.get("result") or {}).get("data")
    if not isinstance(data, dict):
        return None
    if _is_donation(data.get("data")):
        return data["data"]
    if _is_donation(data):
        return data
    return None


class DACollector:
    def __init__(self, store: Store, rates: Rates, on_new: Callable[[], None], client: httpx.AsyncClient):
        self.store = store
        self.rates = rates
        self.on_new = on_new
        self.client = client

    async def ensure_token(self) -> str:
        cfg = self.store.get_setting(SETTINGS_KEY) or {}
        if not cfg.get("access_token"):
            raise RuntimeError("DonationAlerts: нужно войти через OAuth")
        if cfg.get("expires_at", 0) - time.time() < REFRESH_MARGIN and cfg.get("refresh_token"):
            cfg = await refresh(self.client, cfg)
            self.store.set_setting(SETTINGS_KEY, cfg)
        return cfg["access_token"]

    async def _api(self, method: str, path: str, **kw) -> dict:
        token = await self.ensure_token()
        resp = await self.client.request(method, f"{SITE}/api/v1{path}",
                                         headers={"Authorization": f"Bearer {token}"}, timeout=15, **kw)
        resp.raise_for_status()
        return resp.json()

    async def _save(self, obj: dict) -> bool:
        try:
            rub = await self.rates.to_rub(float(obj["amount"]), (obj.get("currency") or "RUB").upper())
            d = parse_da_donation(obj, rub)
        except (KeyError, TypeError, ValueError) as e:
            log.warning("DonationAlerts: unparseable donation %r: %s", obj, e)
            return False
        if d.amount_rub is None:
            log.warning("DonationAlerts: no RUB rate for %s, donation %s saved without amount", d.currency, d.external_id)
        return self.store.add_donation(d)

    async def handle_message(self, msg: dict) -> None:
        obj = extract_push_donation(msg)
        if obj is None:
            res = msg.get("result") or {}
            is_sub_ack = isinstance(res.get("data"), dict) and "info" in res["data"]
            if res.get("channel") and not is_sub_ack:
                log.warning("DonationAlerts: unrecognized push %s", json.dumps(msg, ensure_ascii=False))
            return
        if await self._save(obj):
            self.on_new()

    def catch_up_since(self) -> datetime | None:
        latest = self.store.latest_donation_time("da")
        if latest is not None:
            return latest
        goal = self.store.active_goal()
        return goal.started_at if goal else None

    async def catch_up(self, since: datetime) -> int:
        added = 0
        for page in range(1, MAX_PAGES + 1):
            body = await self._api("GET", "/alerts/donations", params={"page": page})
            reached = False
            for obj in body.get("data") or []:
                try:
                    if parse_created_at(obj["created_at"]) < since:
                        reached = True
                        break
                except (KeyError, TypeError, ValueError):
                    continue
                added += await self._save(obj)
            if reached or not (body.get("links") or {}).get("next"):
                break
        if added:
            self.on_new()
        return added

    async def fix_missing_rub(self) -> int:
        fixed = 0
        for d in self.store.donations_missing_rub():
            rub = await self.rates.to_rub(d.amount, d.currency)
            if rub is not None:
                self.store.set_amount_rub(d.platform, d.external_id, rub)
                fixed += 1
        if fixed:
            self.on_new()
        return fixed

    async def _fix_rub_hourly(self) -> None:
        while True:
            await asyncio.sleep(3600)
            await self.fix_missing_rub()

    async def connect(self, mark_ok: Callable[[], None]) -> None:
        user = (await self._api("GET", "/user/oauth"))["data"]
        channel = f"$alerts:donation_{user['id']}"
        async with connect(CENTRIFUGO_URL, ping_interval=20) as ws:
            await ws.send(json.dumps({"params": {"token": user["socket_connection_token"]}, "id": 1}))
            client_id = None
            while client_id is None:
                for msg in _messages(await asyncio.wait_for(ws.recv(), timeout=15)):
                    if msg.get("id") == 1:
                        if msg.get("error"):
                            raise ConnectionError(f"Centrifugo connect: {msg['error']}")
                        client_id = msg["result"]["client"]
            sub = await self._api("POST", "/centrifuge/subscribe", json={"channels": [channel], "client": client_id})
            chan_token = next(c["token"] for c in sub["channels"] if c["channel"] == channel)
            await ws.send(json.dumps({"params": {"channel": channel, "token": chan_token}, "method": 1, "id": 2}))

            mark_ok()
            since = self.catch_up_since()
            if since is not None:
                await self.catch_up(since)
            await self.fix_missing_rub()

            async def ping() -> None:
                n = 100
                while True:
                    await asyncio.sleep(CENTRIFUGO_PING)
                    n += 1
                    await ws.send(json.dumps({"method": 7, "id": n}))

            tasks = [asyncio.create_task(ping()), asyncio.create_task(self._fix_rub_hourly())]
            try:
                async for frame in ws:
                    for msg in _messages(frame):
                        if msg.get("id") == 2 and msg.get("error"):
                            raise ConnectionError(f"Centrifugo subscribe: {msg['error']}")
                        await self.handle_message(msg)
            finally:
                for task in tasks:
                    task.cancel()


def _messages(frame: str | bytes) -> list[dict]:
    if isinstance(frame, bytes):
        frame = frame.decode()
    out = []
    for line in frame.splitlines():
        if line.strip():
            try:
                out.append(json.loads(line))
            except ValueError:
                log.warning("DonationAlerts: bad frame %r", line)
    return out
