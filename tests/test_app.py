import json

import pytest
from fastapi.testclient import TestClient

from donatehub.app import create_app
from donatehub.store import Store
from donatehub.style import DEFAULT_STYLE, load_presets, merge_style


@pytest.fixture
def client():
    app = create_app(Store(":memory:"), start_collectors=False)
    with TestClient(app, base_url="http://127.0.0.1:8000") as c:
        yield c


def test_ws_receives_update_after_debug_donation(client):
    assert client.post("/api/goal", json={"title": "Микрофон", "target_rub": 10000}).status_code == 200
    with client.websocket_connect("ws://127.0.0.1:8000/ws") as ws:
        first = ws.receive_json()
        assert first["collected_rub"] == 0 and first["goal"]["title"] == "Микрофон"
        assert client.post("/api/debug/donation", json={"amount_rub": 500}).status_code == 200
        assert ws.receive_json()["collected_rub"] == 500


def test_test_donation_ignores_goal_start_and_expires(client):
    client.app.state.hub.test_ttl = 0.2
    future = "2099-01-01T00:00:00+00:00"
    assert client.post("/api/goal", json={"title": "Микрофон", "target_rub": 10000, "started_at": future}).status_code == 200
    with client.websocket_connect("ws://127.0.0.1:8000/ws") as ws:
        assert ws.receive_json()["collected_rub"] == 0
        r = client.post("/api/debug/donation", json={"amount_rub": 100})
        assert r.status_code == 200 and r.json()["collected_rub"] == 100
        assert ws.receive_json()["collected_rub"] == 100
        assert ws.receive_json()["collected_rub"] == 0  # pushed again once the test amount expires
    assert client.app.state.collectors.store.collected(client.app.state.collectors.store.active_goal()) == 0


def test_style_merge_rejects_unknown_and_bad_types(client):
    r = client.put("/api/style", json={"bar": {"height_vw": "x", "nope": 1, "radius_vw": 2}})
    assert r.status_code == 200
    style = client.get("/api/state").json()["style"]
    assert style["bar"]["height_vw"] == 7
    assert style["bar"]["radius_vw"] == 2
    assert "nope" not in style["bar"]


def test_merge_style_rejects_bad_enum_and_css_injection():
    s = merge_style(DEFAULT_STYLE, {"elements": {"title_pos": "sideways"},
                                    "fonts": {"title": {"color": "red;} body{display:none", "family": "Roboto"}}})
    assert s["elements"]["title_pos"] == DEFAULT_STYLE["elements"]["title_pos"]
    assert s["fonts"]["title"]["color"] == DEFAULT_STYLE["fonts"]["title"]["color"]
    assert s["fonts"]["title"]["family"] == "Roboto"


def test_merge_style_int_accepted_for_float():
    s = merge_style(DEFAULT_STYLE, {"bar": {"height_vw": 5}})
    assert s["bar"]["height_vw"] == 5


def test_presets_are_complete_styles():
    presets = load_presets()
    assert len(presets) >= 5
    for p in presets:
        assert p["name"]
        assert p["style"].keys() == DEFAULT_STYLE.keys()


def test_tokens_not_exposed(client):
    assert client.put("/api/platforms/dx", json={"token": "SECRET"}).status_code == 200
    assert client.put("/api/platforms/da", json={"client_id": "7", "client_secret": "SECRET2"}).status_code == 200
    state = client.get("/api/state").json()
    assert "SECRET" not in json.dumps(state)
    assert state["platforms"]["dx"]["configured"] is True
    assert state["platforms"]["da"]["configured"] is False
    assert state["platforms"]["da"]["has_client"] is True
    with client.websocket_connect("ws://127.0.0.1:8000/ws") as ws:
        assert "SECRET" not in json.dumps(ws.receive_json())


def test_goal_lifecycle(client):
    assert client.post("/api/goal", json={"title": "A", "target_rub": 1000}).status_code == 200
    assert client.post("/api/goal", json={"title": "B", "target_rub": 1000}).status_code == 409
    r = client.patch("/api/goal", json={"manual_adjust_rub": 150, "title": "A2"})
    assert r.status_code == 200
    state = client.get("/api/state").json()
    assert state["collected_rub"] == 150 and state["goal"]["title"] == "A2"
    assert client.post("/api/goal/close").status_code == 200
    hist = client.get("/api/history").json()
    assert len(hist) == 1 and hist[0]["title"] == "A2" and hist[0]["collected_rub"] == 150
    assert client.get("/api/state").json()["goal"] is None


def test_goal_with_explicit_dates(client):
    r = client.post("/api/goal", json={"title": "A", "target_rub": 1000,
                                       "started_at": "2026-11-20T10:00:00+03:00",
                                       "ends_at": "2026-11-30T10:00:00Z"})
    assert r.status_code == 200
    g = client.get("/api/state").json()["goal"]
    assert g["started_at"].startswith("2026-11-20T07:00:00")
    assert client.patch("/api/goal", json={"ends_at": None}).status_code == 200
    assert client.get("/api/state").json()["goal"]["ends_at"] is None


def test_goal_validation(client):
    assert client.post("/api/goal", json={"title": "", "target_rub": 1000}).status_code == 422
    assert client.post("/api/goal", json={"title": "A", "target_rub": 0}).status_code == 422
    assert client.patch("/api/goal", json={"title": "x"}).status_code == 404


def test_pages_served(client):
    assert client.get("/overlay").status_code == 200
    assert client.get("/").status_code == 200


def test_da_start_requires_client(client):
    r = client.get("/auth/da/start", follow_redirects=False)
    assert r.status_code == 400
    client.put("/api/platforms/da", json={"client_id": "7", "client_secret": "s"})
    r = client.get("/auth/da/start", follow_redirects=False)
    assert r.status_code in (302, 307)
    assert r.headers["location"].startswith("https://www.donationalerts.com/oauth/authorize")


def test_da_callback_rejects_bad_state(client):
    client.put("/api/platforms/da", json={"client_id": "7", "client_secret": "s"})
    r = client.get("/auth/da/callback?code=abc&state=forged", follow_redirects=False)
    assert r.status_code == 400


def test_text_align_option():
    assert DEFAULT_STYLE["elements"]["text_align"] == "left"
    assert merge_style(DEFAULT_STYLE, {"elements": {"text_align": "center"}})["elements"]["text_align"] == "center"
    assert merge_style(DEFAULT_STYLE, {"elements": {"text_align": "diagonal"}})["elements"]["text_align"] == "left"


def test_cross_site_post_rejected(client):
    client.post("/api/goal", json={"title": "A", "target_rub": 1000})
    r = client.post("/api/goal/close", headers={"Origin": "https://evil.example"})
    assert r.status_code == 403
    assert client.get("/api/state").json()["goal"] is not None
    assert client.post("/api/goal/close", headers={"Origin": "http://127.0.0.1:8000"}).status_code == 200


def test_foreign_host_rejected(client):
    assert client.get("/api/state", headers={"Host": "rebind.evil.example"}).status_code == 400
    assert client.get("/api/state", headers={"Host": "localhost:8000"}).status_code == 200
    assert client.get("/api/state", headers={"Host": "127.0.0.1:8000"}).status_code == 200


class FakeCollector:
    def __init__(self):
        self.calls = []

    async def catch_up(self, since):
        self.calls.append(since)
        return 0


def test_backdated_goal_triggers_backfill():
    app = create_app(Store(":memory:"), start_collectors=False)
    fake = FakeCollector()
    app.state.collectors.instances["dx"] = fake
    with TestClient(app, base_url="http://127.0.0.1:8000") as c:
        c.post("/api/goal", json={"title": "A", "target_rub": 1000, "started_at": "2026-11-01T00:00:00Z"})
        c.patch("/api/goal", json={"started_at": "2026-10-01T00:00:00Z"})
        c.patch("/api/goal", json={"title": "B"})
    assert [d.isoformat() for d in fake.calls] == ["2026-11-01T00:00:00+00:00", "2026-10-01T00:00:00+00:00"]


def test_redirect_uri_follows_port():
    app = create_app(Store(":memory:"), start_collectors=False, port=50001)
    with TestClient(app, base_url="http://127.0.0.1:50001") as c:
        state = c.get("/api/state").json()
        assert state["platforms"]["da"]["redirect_uri"] == "http://127.0.0.1:50001/auth/da/callback"
        assert state["port"]["current"] == 50001
        c.put("/api/platforms/da", json={"client_id": "7", "client_secret": "s"})
        loc = c.get("/auth/da/start", follow_redirects=False).headers["location"]
        assert "127.0.0.1%3A50001%2Fauth%2Fda%2Fcallback" in loc


def test_port_setting_validation(client):
    assert client.put("/api/settings/port", json={"port": 70000}).status_code == 422
    assert client.put("/api/settings/port", json={"port": 80}).status_code == 422
    assert client.put("/api/settings/port", json={"port": 48000}).status_code == 200
    port = client.get("/api/state").json()["port"]
    assert port["saved"] == 48000 and port["current"] == 47321


def test_da_callback_success_page(client, monkeypatch):
    from donatehub.collectors import donationalerts as da

    async def fake_exchange(http, cfg, code, redirect):
        assert code == "abc" and redirect.endswith(":47321/auth/da/callback")
        return {**cfg, "access_token": "AT", "refresh_token": "RT", "expires_at": 9e18}

    monkeypatch.setattr(da, "exchange_code", fake_exchange)
    client.put("/api/platforms/da", json={"client_id": "7", "client_secret": "s"})
    loc = client.get("/auth/da/start", follow_redirects=False).headers["location"]
    state = loc.split("state=")[1].split("&")[0]
    r = client.get(f"/auth/da/callback?code=abc&state={state}", follow_redirects=False)
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/html")
    assert "подключён" in r.text
    assert client.get("/api/state").json()["platforms"]["da"]["configured"] is True
