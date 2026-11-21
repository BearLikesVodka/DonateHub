from datetime import UTC, datetime, timedelta

import httpx

from donation_goal.currency import RateProvider

CBR = {"Valute": {"USD": {"Value": 90.0, "Nominal": 1}, "KZT": {"Value": 18.0, "Nominal": 100}}}


def client_with(handler):
    return httpx.AsyncClient(transport=httpx.MockTransport(handler))


async def test_rub_passthrough(store):
    calls = []
    rp = RateProvider(store, client_with(lambda r: calls.append(r) or httpx.Response(200, json=CBR)))
    assert await rp.to_rub(150, "RUB") == 150
    assert calls == []


async def test_convert_fetches_and_caches(store):
    calls = []
    rp = RateProvider(store, client_with(lambda r: calls.append(r) or httpx.Response(200, json=CBR)))
    assert await rp.to_rub(2, "USD") == 180
    assert await rp.to_rub(1000, "KZT") == 180
    assert len(calls) == 1


async def test_convert_uses_cached_rate_when_offline(store):
    old = (datetime.now(UTC) - timedelta(days=3)).isoformat()
    store.set_setting("cbr_rates", {"fetched_at": old, "rates": {"USD": 80.0}})

    def boom(request):
        raise httpx.ConnectError("offline")

    rp = RateProvider(store, client_with(boom))
    assert await rp.to_rub(1, "USD") == 80


async def test_unknown_currency_returns_none(store):
    def boom(request):
        raise httpx.ConnectError("offline")

    rp = RateProvider(store, client_with(boom))
    assert await rp.to_rub(1, "USD") is None
