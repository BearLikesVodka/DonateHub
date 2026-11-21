import logging
from datetime import UTC, datetime, timedelta

import httpx

from .store import Store

log = logging.getLogger(__name__)

CBR_URL = "https://www.cbr-xml-daily.ru/daily_json.js"
CACHE_KEY = "cbr_rates"
MAX_AGE = timedelta(hours=24)


class RateProvider:
    """RUB per unit of a currency, from the Central Bank of Russia daily rates; the last good rates survive outages."""

    def __init__(self, store: Store, client: httpx.AsyncClient, url: str = CBR_URL):
        self.store = store
        self.client = client
        self.url = url

    def _cached(self) -> dict | None:
        return self.store.get_setting(CACHE_KEY)

    async def _refresh(self) -> None:
        try:
            resp = await self.client.get(self.url, timeout=10)
            resp.raise_for_status()
            # The endpoint is served as JS but the body is plain JSON.
            valute = resp.json()["Valute"]
            rates = {code: v["Value"] / v["Nominal"] for code, v in valute.items()}
        except (httpx.HTTPError, ValueError, KeyError, TypeError, ZeroDivisionError) as e:
            log.warning("CBR rates fetch failed: %s", e)
            return
        self.store.set_setting(CACHE_KEY, {"fetched_at": datetime.now(UTC).isoformat(), "rates": rates})

    async def to_rub(self, amount: float, currency: str) -> float | None:
        currency = currency.upper()
        if currency == "RUB":
            return amount
        cached = self._cached()
        fresh = cached and datetime.now(UTC) - datetime.fromisoformat(cached["fetched_at"]) < MAX_AGE
        if not fresh or currency not in cached["rates"]:
            await self._refresh()
            cached = self._cached()
        rate = cached and cached["rates"].get(currency)
        if rate is None:
            log.warning("no RUB rate for %s", currency)
            return None
        return round(amount * rate, 2)
