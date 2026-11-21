"""Minimal ASP.NET Core SignalR client (JSON protocol over WebSockets) — enough to receive hub invocations."""

import asyncio
import json
from collections.abc import AsyncIterator, Awaitable, Callable
from typing import Any
from urllib.parse import urlencode, urlsplit, urlunsplit

import httpx
from websockets.asyncio.client import connect

RS = "\x1e"
PING_INTERVAL = 15

INVOCATION, PING, CLOSE = 1, 6, 7


def encode(msg: dict) -> str:
    return json.dumps(msg, separators=(",", ":")) + RS


def decode(buf: str) -> list[dict]:
    return [json.loads(part) for part in buf.split(RS) if part.strip()]


def _with_query(url: str, **params: str) -> str:
    parts = urlsplit(url)
    query = "&".join(q for q in (parts.query, urlencode(params)) if q)
    return urlunsplit(parts._replace(query=query))


def _ws_url(url: str) -> str:
    parts = urlsplit(url)
    return urlunsplit(parts._replace(scheme={"https": "wss", "http": "ws"}.get(parts.scheme, parts.scheme)))


async def signalr_stream(
    hub_url: str,
    token: str,
    client: httpx.AsyncClient,
    on_open: Callable[[], Awaitable[None]] | None = None,
) -> AsyncIterator[tuple[str, list[Any]]]:
    neg_url = hub_url.rstrip("/") + "/negotiate"
    resp = await client.post(_with_query(neg_url, negotiateVersion="1", access_token=token), timeout=15)
    resp.raise_for_status()
    neg = resp.json()
    if "url" in neg:  # redirect to a SignalR service
        hub_url, token = neg["url"], neg.get("accessToken", token)
        resp = await client.post(_with_query(hub_url.split("?")[0].rstrip("/") + "/negotiate", negotiateVersion="1",
                                             access_token=token), timeout=15)
        resp.raise_for_status()
        neg = resp.json()
    conn_id = neg.get("connectionToken") or neg["connectionId"]

    async with connect(_with_query(_ws_url(hub_url), id=conn_id, access_token=token), ping_interval=None) as ws:
        await ws.send(encode({"protocol": "json", "version": 1}))
        handshake = decode(await asyncio.wait_for(ws.recv(), timeout=15))
        if not handshake or handshake[0].get("error"):
            raise ConnectionError(f"SignalR handshake failed: {handshake}")

        async def keepalive():
            while True:
                await asyncio.sleep(PING_INTERVAL)
                await ws.send(encode({"type": PING}))

        pinger = asyncio.create_task(keepalive())
        try:
            if on_open is not None:
                await on_open()
            # Messages after the handshake may arrive in the same frame as the handshake response.
            pending = handshake[1:]
            while True:
                for msg in pending:
                    match msg.get("type"):
                        case 1:
                            yield msg.get("target", ""), msg.get("arguments") or []
                        case 7:
                            raise ConnectionError(f"SignalR closed: {msg.get('error') or 'no reason'}")
                pending = decode(await asyncio.wait_for(ws.recv(), timeout=PING_INTERVAL * 4))
        finally:
            pinger.cancel()
