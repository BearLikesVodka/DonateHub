from datetime import UTC, datetime, timedelta

import httpx

from conftest import t
from donatehub.collectors.donatex import DonateXCollector, parse_dx_donation

DOC_EXAMPLE = {
    "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
    "username": "Viewer",
    "message": "hi",
    "currency": "USD",
    "amount": 100,
    "amountInRub": 9200,
    "timestamp": "2026-10-12T18:42:07Z",
    "wasShown": False,
    "isTest": False,
}


def dx(id, at: datetime, rub=100.0, is_test=False):
    return {**DOC_EXAMPLE, "id": id, "amountInRub": rub, "amount": rub, "currency": "RUB",
            "timestamp": at.strftime("%Y-%m-%dT%H:%M:%SZ"), "isTest": is_test}


def test_parse_dx_donation():
    d = parse_dx_donation(DOC_EXAMPLE)
    assert d.platform == "dx"
    assert d.external_id == "3fa85f64-5717-4562-b3fc-2c963f66afa6"
    assert d.amount == 100 and d.currency == "USD" and d.amount_rub == 9200
    assert d.created_at == datetime(2026, 10, 12, 18, 42, 7, tzinfo=UTC)
    assert d.is_test is False
    assert d.username == "Viewer"


def test_rerun_is_deduplicated(store):
    calls = []
    c = DonateXCollector(store, "tok", lambda: calls.append(1), httpx.AsyncClient())
    c.handle_event("DonationCreated", [dx("a", t(12))])
    c.handle_event("DonationRerun", [dx("a", t(12))])
    c.handle_event("GoalUpdated", [None])
    assert len(calls) == 1
    assert store.latest_donation_time("dx") == t(12)


async def test_catchup_stops_at_boundary(store):
    since = t(10)
    newest = t(20)
    page = [dx(f"d{i}", newest - timedelta(minutes=10 * i)) for i in range(100)]
    requests = []

    def handler(request):
        requests.append(request)
        return httpx.Response(200, json=page)

    calls = []
    c = DonateXCollector(store, "tok", lambda: calls.append(1), httpx.AsyncClient(transport=httpx.MockTransport(handler)))
    added = await c.catch_up(since)

    assert len(requests) == 1
    r = requests[0]
    assert r.headers["Authorization"] == "Bearer tok"
    assert r.url.params["sortOrder"] == "NewestFirst"
    assert r.url.params["take"] == "100"
    assert r.url.params["skip"] == "0"
    expected = sum(1 for d in page if datetime.fromisoformat(d["timestamp"]) >= since)
    assert added == expected == 61
    assert calls == [1]


async def test_catchup_pages_until_short_page(store):
    pages = {0: [dx(f"a{i}", t(20) - timedelta(seconds=i)) for i in range(100)],
             100: [dx("b", t(15))]}
    seen = []

    def handler(request):
        skip = int(request.url.params["skip"])
        seen.append(skip)
        return httpx.Response(200, json=pages.get(skip, []))

    c = DonateXCollector(store, "tok", lambda: None, httpx.AsyncClient(transport=httpx.MockTransport(handler)))
    assert await c.catch_up(t(10)) == 101
    assert seen == [0, 100]


def test_catchup_since_prefers_latest_known(store):
    c = DonateXCollector(store, "tok", lambda: None, httpx.AsyncClient())
    assert c.catch_up_since() is None
    store.create_goal("A", 1000, t(8), None)
    assert c.catch_up_since() == t(8)
    c.handle_event("DonationCreated", [dx("x", t(11))])
    assert c.catch_up_since() == t(11)
