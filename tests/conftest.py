from datetime import UTC, datetime

import pytest

from donation_goal.models import Donation
from donation_goal.store import Store


def t(h: int, m: int = 0) -> datetime:
    return datetime(2026, 11, 21, h, m, tzinfo=UTC)


def make_donation(platform="dx", external_id="x", rub=100.0, at=None, is_test=False,
                  amount=None, currency="RUB") -> Donation:
    return Donation(
        platform=platform,
        external_id=external_id,
        username="viewer",
        amount=amount if amount is not None else (rub or 0.0),
        currency=currency,
        amount_rub=rub,
        created_at=at or t(12),
        is_test=is_test,
        raw={},
    )


@pytest.fixture
def store():
    return Store(":memory:")
