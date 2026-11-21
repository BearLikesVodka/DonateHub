from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Any, Literal

Platform = Literal["da", "dx", "manual"]


@dataclass(frozen=True)
class Donation:
    platform: Platform
    external_id: str
    username: str
    amount: float
    currency: str
    amount_rub: float | None
    created_at: datetime
    is_test: bool
    raw: dict[str, Any] = field(default_factory=dict, compare=False)


@dataclass
class Goal:
    id: int
    title: str
    target_rub: float
    started_at: datetime
    ends_at: datetime | None
    closed_at: datetime | None
    manual_adjust_rub: float


def to_iso(dt: datetime) -> str:
    """Normalize to the single UTC string format stored in the DB, so string comparison orders correctly."""
    if dt.tzinfo is None:
        raise ValueError("naive datetime")
    return dt.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%S.%f+00:00")


def from_iso(s: str) -> datetime:
    return datetime.fromisoformat(s).astimezone(UTC)
