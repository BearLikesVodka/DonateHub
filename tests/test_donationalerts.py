import json
import time
from datetime import UTC, datetime, timedelta
from urllib.parse import parse_qs, urlsplit

import httpx

from conftest import make_donation, t
from donation_goal.collectors.donationalerts import (
    DACollector,
    authorize_url,
    extract_push_donation,
    parse_created_at,
    parse_da_donation,
    redirect_uri,
)


class FakeRates:
    def __init__(self, rate=90.0):
        self.rate = rate

    async def to_rub(self, amount, currency):
        if currency == "RUB":
            return amount
        return None if self.rate is None else amount * self.rate


def da(id, at: datetime, amount=100, currency="RUB"):
    return {"id": id, "name": "donation", "username": "Viewer", "message_type": "text", "message": "hi",
            "amount": amount, "currency": currency, "is_shown": 1,
            "created_at": at.strftime("%Y-%m-%d %H:%M:%S"), "shown_at": None}


def configure(store, **over):
    cfg = {"client_id": "7", "client_secret": "sec", "access_token": "AT", "refresh_token": "RT",
           "expires_at": time.time() + 365 * 86400}
    cfg.update(over)
    store.set_setting("da", cfg)


def test_parse_da_created_at_formats():
    expected = datetime(2026, 11, 21, 9, 5, 7, tzinfo=UTC)
    assert parse_created_at("2026-11-21 09.05.07") == expected
    assert parse_created_at("2026-11-21 09:05:07") == expected


def test_parse_da_donation():
    d = parse_da_donation(da(5, t(12), amount=2, currency="USD"), 180.0)
    assert d.platform == "da" and d.external_id == "5"
    assert d.amount == 2 and d.currency == "USD" and d.amount_rub == 180.0
    assert d.created_at == t(12) and d.is_test is False


def test_extract_push_donation():
    push = {"result": {"channel": "$alerts:donation_3", "data": {"seq": 1, "data": da(5, t(12))}}}
    assert extract_push_donation(push)["id"] == 5
    flat = {"result": {"channel": "$alerts:donation_3", "data": da(6, t(12))}}
    assert extract_push_donation(flat)["id"] == 6
    sub_ack = {"result": {"type": 1, "channel": "$alerts:donation_3", "data": {"info": {"user": "1"}}}}
    assert extract_push_donation(sub_ack) is None
    assert extract_push_donation({"id": 1, "result": {"client": "x"}}) is None


def test_authorize_url():
    assert redirect_uri(47321) == "http://127.0.0.1:47321/auth/da/callback"
    u = urlsplit(authorize_url("7", "st", redirect_uri(47321)))
    assert u.netloc == "www.donationalerts.com" and u.path == "/oauth/authorize"
    q = parse_qs(u.query)
    assert q["client_id"] == ["7"] and q["response_type"] == ["code"] and q["state"] == ["st"]
    assert q["redirect_uri"] == ["http://127.0.0.1:47321/auth/da/callback"]
    assert q["scope"] == ["oauth-user-show oauth-donation-subscribe oauth-donation-index"]


async def test_refresh_keeps_old_refresh_token(store):
    configure(store, expires_at=time.time() + 60)
    seen = []

    def handler(request):
        seen.append(parse_qs(request.content.decode()))
        return httpx.Response(200, json={"token_type": "Bearer", "access_token": "NEW", "expires_in": 3600})

    c = DACollector(store, FakeRates(), lambda: None, httpx.AsyncClient(transport=httpx.MockTransport(handler)))
    assert await c.ensure_token() == "NEW"
    cfg = store.get_setting("da")
    assert cfg["access_token"] == "NEW" and cfg["refresh_token"] == "RT"
    assert cfg["expires_at"] > time.time() + 3000
    assert seen[0]["grant_type"] == ["refresh_token"] and seen[0]["refresh_token"] == ["RT"]


async def test_ensure_token_no_refresh_when_fresh(store):
    configure(store)

    def handler(request):
        raise AssertionError("should not refresh")

    c = DACollector(store, FakeRates(), lambda: None, httpx.AsyncClient(transport=httpx.MockTransport(handler)))
    assert await c.ensure_token() == "AT"


async def test_catchup_stops_at_boundary(store):
    configure(store)
    page1 = [da(100 - i, t(20) - timedelta(minutes=30 * i)) for i in range(30)]
    page1[1] = da(99, t(19, 30), amount=2, currency="USD")
    requests = []

    def handler(request):
        requests.append(request)
        return httpx.Response(200, json={"data": page1, "links": {"next": "https://x/?page=2"}, "meta": {}})

    calls = []
    c = DACollector(store, FakeRates(), lambda: calls.append(1), httpx.AsyncClient(transport=httpx.MockTransport(handler)))
    added = await c.catch_up(t(10))
    assert len(requests) == 1
    assert requests[0].headers["Authorization"] == "Bearer AT"
    assert requests[0].url.params["page"] == "1"
    assert added == 21  # t(20) back to t(10) inclusive, every 30 minutes
    g = store.create_goal("A", 1, t(19), None)
    assert store.collected(g) == 100 + 180 + 100  # t(20), t(19:30) USD, t(19)
    assert calls == [1]


async def test_catchup_follows_next_link(store):
    configure(store)
    pages = {"1": ([da(2, t(15))], "https://x/?page=2"), "2": ([da(1, t(14))], None)}

    def handler(request):
        data, nxt = pages[request.url.params["page"]]
        return httpx.Response(200, json={"data": data, "links": {"next": nxt}})

    c = DACollector(store, FakeRates(), lambda: None, httpx.AsyncClient(transport=httpx.MockTransport(handler)))
    assert await c.catch_up(t(10)) == 2


async def test_fix_missing_rub(store):
    store.add_donation(make_donation("da", "u1", rub=None, amount=2, currency="USD"))
    calls = []
    c = DACollector(store, FakeRates(90.0), lambda: calls.append(1), httpx.AsyncClient())
    assert await c.fix_missing_rub() == 1
    assert store.donations_missing_rub() == []
    assert calls == [1]


async def test_push_saves_donation_and_converts(store):
    calls = []
    c = DACollector(store, FakeRates(None), lambda: calls.append(1), httpx.AsyncClient())
    msg = {"result": {"channel": "$alerts:donation_3", "data": {"data": da(7, t(12), amount=5, currency="EUR")}}}
    await c.handle_message(msg)
    await c.handle_message(msg)
    assert calls == [1]
    [d] = store.donations_missing_rub()
    assert d.external_id == "7"
