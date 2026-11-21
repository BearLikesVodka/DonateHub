import pytest

from conftest import make_donation, t


def test_add_donation_dedup(store):
    d = make_donation("dx", "a1", rub=100)
    assert store.add_donation(d) is True
    assert store.add_donation(d) is False


def test_same_id_different_platforms_both_kept(store):
    assert store.add_donation(make_donation("dx", "1")) is True
    assert store.add_donation(make_donation("da", "1")) is True


def test_collected_window_and_tests(store):
    g = store.create_goal("Микрофон", 10000, t(10), None)
    store.add_donation(make_donation("dx", "before", rub=500, at=t(9)))
    store.add_donation(make_donation("dx", "in", rub=300, at=t(11)))
    store.add_donation(make_donation("da", "in2", rub=200, at=t(12)))
    store.add_donation(make_donation("dx", "test", rub=999, at=t(12), is_test=True))
    store.add_donation(make_donation("da", "norub", rub=None, at=t(12)))
    assert store.collected(g) == 500
    store.update_goal(g.id, manual_adjust_rub=50)
    assert store.collected(store.active_goal()) == 550


def test_ends_at_limits_window(store):
    g = store.create_goal("A", 1000, t(10), t(12))
    store.add_donation(make_donation("dx", "in", rub=100, at=t(11)))
    store.add_donation(make_donation("dx", "after", rub=100, at=t(13)))
    assert store.collected(g) == 100


def test_closed_goal_window_and_history(store):
    g = store.create_goal("A", 1000, t(10), None)
    store.add_donation(make_donation("dx", "x", rub=100, at=t(11)))
    store.close_goal(g.id, t(12))
    store.add_donation(make_donation("dx", "y", rub=100, at=t(13)))
    assert store.active_goal() is None
    [(hg, total)] = store.history()
    assert hg.title == "A" and total == 100


def test_single_active_goal(store):
    store.create_goal("A", 1, t(1), None)
    with pytest.raises(ValueError):
        store.create_goal("B", 1, t(2), None)


def test_settings_roundtrip(store):
    store.set_setting("style", {"a": 1})
    assert store.get_setting("style") == {"a": 1}
    assert store.get_setting("missing", 5) == 5


def test_missing_rub_and_set_amount(store):
    store.add_donation(make_donation("da", "u1", rub=None, amount=2, currency="USD"))
    [d] = store.donations_missing_rub()
    assert d.currency == "USD" and d.amount == 2
    store.set_amount_rub("da", "u1", 180)
    assert store.donations_missing_rub() == []


def test_latest_donation_time(store):
    assert store.latest_donation_time("dx") is None
    store.add_donation(make_donation("dx", "a", at=t(9)))
    store.add_donation(make_donation("dx", "b", at=t(11)))
    store.add_donation(make_donation("da", "c", at=t(15)))
    assert store.latest_donation_time("dx") == t(11)
