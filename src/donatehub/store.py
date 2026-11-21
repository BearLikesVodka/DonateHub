import json
import sqlite3
from datetime import datetime
from pathlib import Path
from typing import Any

from .models import Donation, Goal, from_iso, to_iso

SCHEMA = """
CREATE TABLE IF NOT EXISTS donations (
    platform TEXT NOT NULL,
    external_id TEXT NOT NULL,
    username TEXT NOT NULL,
    amount REAL NOT NULL,
    currency TEXT NOT NULL,
    amount_rub REAL,
    created_at TEXT NOT NULL,
    is_test INTEGER NOT NULL,
    raw_json TEXT NOT NULL,
    PRIMARY KEY (platform, external_id)
);
CREATE INDEX IF NOT EXISTS donations_created ON donations(created_at);
CREATE TABLE IF NOT EXISTS goals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    target_rub REAL NOT NULL,
    started_at TEXT NOT NULL,
    ends_at TEXT,
    closed_at TEXT,
    manual_adjust_rub REAL NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);
"""

_GOAL_FIELDS = {"title", "target_rub", "started_at", "ends_at", "manual_adjust_rub"}


def _opt_iso(dt: datetime | None) -> str | None:
    return to_iso(dt) if dt is not None else None


def _opt_dt(s: str | None) -> datetime | None:
    return from_iso(s) if s is not None else None


class Store:
    def __init__(self, path: str | Path):
        self.db = sqlite3.connect(str(path), check_same_thread=False, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.executescript(SCHEMA)

    # donations

    def add_donation(self, d: Donation) -> bool:
        cur = self.db.execute(
            "INSERT OR IGNORE INTO donations VALUES (?,?,?,?,?,?,?,?,?)",
            (d.platform, d.external_id, d.username, d.amount, d.currency, d.amount_rub,
             to_iso(d.created_at), int(d.is_test), json.dumps(d.raw, ensure_ascii=False)),
        )
        return cur.rowcount == 1

    def set_amount_rub(self, platform: str, external_id: str, amount_rub: float) -> None:
        self.db.execute(
            "UPDATE donations SET amount_rub=? WHERE platform=? AND external_id=?",
            (amount_rub, platform, external_id),
        )

    def donations_missing_rub(self) -> list[Donation]:
        rows = self.db.execute("SELECT * FROM donations WHERE amount_rub IS NULL").fetchall()
        return [self._donation(r) for r in rows]

    def latest_donation_time(self, platform: str) -> datetime | None:
        row = self.db.execute(
            "SELECT MAX(created_at) FROM donations WHERE platform=?", (platform,)
        ).fetchone()
        return _opt_dt(row[0])

    @staticmethod
    def _donation(r: sqlite3.Row) -> Donation:
        return Donation(
            platform=r["platform"], external_id=r["external_id"], username=r["username"],
            amount=r["amount"], currency=r["currency"], amount_rub=r["amount_rub"],
            created_at=from_iso(r["created_at"]), is_test=bool(r["is_test"]),
            raw=json.loads(r["raw_json"]),
        )

    # goals

    @staticmethod
    def _goal(r: sqlite3.Row) -> Goal:
        return Goal(
            id=r["id"], title=r["title"], target_rub=r["target_rub"],
            started_at=from_iso(r["started_at"]), ends_at=_opt_dt(r["ends_at"]),
            closed_at=_opt_dt(r["closed_at"]), manual_adjust_rub=r["manual_adjust_rub"],
        )

    def get_goal(self, goal_id: int) -> Goal:
        row = self.db.execute("SELECT * FROM goals WHERE id=?", (goal_id,)).fetchone()
        if row is None:
            raise KeyError(goal_id)
        return self._goal(row)

    def active_goal(self) -> Goal | None:
        row = self.db.execute(
            "SELECT * FROM goals WHERE closed_at IS NULL ORDER BY id DESC LIMIT 1"
        ).fetchone()
        return self._goal(row) if row else None

    def create_goal(self, title: str, target_rub: float, started_at: datetime,
                    ends_at: datetime | None) -> Goal:
        if self.active_goal() is not None:
            raise ValueError("active goal already exists")
        cur = self.db.execute(
            "INSERT INTO goals (title, target_rub, started_at, ends_at) VALUES (?,?,?,?)",
            (title, target_rub, to_iso(started_at), _opt_iso(ends_at)),
        )
        return self.get_goal(cur.lastrowid)

    def update_goal(self, goal_id: int, **fields: Any) -> Goal:
        unknown = set(fields) - _GOAL_FIELDS
        if unknown:
            raise ValueError(f"unknown goal fields: {unknown}")
        for name, value in fields.items():
            if isinstance(value, datetime) or (value is None and name == "ends_at"):
                value = _opt_iso(value)
            self.db.execute(f"UPDATE goals SET {name}=? WHERE id=?", (value, goal_id))
        return self.get_goal(goal_id)

    def close_goal(self, goal_id: int, closed_at: datetime) -> Goal:
        self.db.execute("UPDATE goals SET closed_at=? WHERE id=?", (to_iso(closed_at), goal_id))
        return self.get_goal(goal_id)

    def collected(self, goal: Goal) -> float:
        ends = [x for x in (goal.closed_at, goal.ends_at) if x is not None]
        end = to_iso(min(ends)) if ends else "9999"
        row = self.db.execute(
            "SELECT COALESCE(SUM(amount_rub), 0) FROM donations "
            "WHERE is_test=0 AND amount_rub IS NOT NULL AND created_at >= ? AND created_at < ?",
            (to_iso(goal.started_at), end),
        ).fetchone()
        return row[0] + goal.manual_adjust_rub

    def history(self) -> list[tuple[Goal, float]]:
        rows = self.db.execute(
            "SELECT * FROM goals WHERE closed_at IS NOT NULL ORDER BY started_at DESC, id DESC"
        ).fetchall()
        return [(g, self.collected(g)) for g in map(self._goal, rows)]

    # settings

    def get_setting(self, key: str, default: Any = None) -> Any:
        row = self.db.execute("SELECT value_json FROM settings WHERE key=?", (key,)).fetchone()
        return json.loads(row[0]) if row else default

    def set_setting(self, key: str, value: Any) -> None:
        self.db.execute(
            "INSERT INTO settings VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
            (key, json.dumps(value, ensure_ascii=False)),
        )
