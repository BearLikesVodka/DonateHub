import asyncio
import logging
import secrets
from contextlib import asynccontextmanager
from datetime import UTC, datetime
from pathlib import Path
from typing import Annotated
from urllib.parse import urlsplit

import httpx
from fastapi import Body, FastAPI, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.middleware.trustedhost import TrustedHostMiddleware
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse, RedirectResponse
from fastapi.staticfiles import StaticFiles
from pydantic import AwareDatetime, BaseModel, Field

from .collectors import donationalerts as da
from .collectors.base import CollectorStatus, describe_error, run_forever
from .collectors.donatex import DonateXCollector
from .config import DEFAULT_PORT, resolve_port
from .currency import RateProvider
from .hub import Hub
from .models import to_iso
from .store import Store
from .style import load_presets, merge_style

log = logging.getLogger(__name__)
STATIC = Path(__file__).parent / "static"
LOCAL_HOSTS = ["127.0.0.1", "localhost"]
SAFE_METHODS = {"GET", "HEAD", "OPTIONS"}


class GoalCreate(BaseModel):
    title: str = Field(min_length=1, max_length=200)
    target_rub: float = Field(gt=0)
    started_at: AwareDatetime | None = None
    ends_at: AwareDatetime | None = None


class GoalPatch(BaseModel):
    title: str | None = Field(default=None, min_length=1, max_length=200)
    target_rub: float | None = Field(default=None, gt=0)
    started_at: AwareDatetime | None = None
    ends_at: AwareDatetime | None = None
    manual_adjust_rub: float | None = None


class DebugDonation(BaseModel):
    amount_rub: float = Field(gt=0, le=1_000_000)


class PortSettings(BaseModel):
    port: int = Field(ge=1024, le=65535)


class DXSettings(BaseModel):
    token: str = Field(min_length=1)


class DASettings(BaseModel):
    client_id: str = Field(min_length=1)
    client_secret: str = Field(min_length=1)


class Collectors:
    def __init__(self, store: Store, hub: Hub, port: int):
        self.store = store
        self.port = port
        self.hub = hub
        self.client: httpx.AsyncClient | None = None
        self.rates: RateProvider | None = None
        self.status = {"da": CollectorStatus(), "dx": CollectorStatus()}
        self.tasks: dict[str, asyncio.Task] = {}
        self.instances: dict[str, object] = {}
        self._backfills: set[asyncio.Task] = set()
        self.enabled = False

    async def start(self) -> None:
        self.enabled = True
        self.client = httpx.AsyncClient()
        self.rates = RateProvider(self.store, self.client)
        self.restart("da")
        self.restart("dx")

    async def stop(self) -> None:
        for task in self.tasks.values():
            task.cancel()
        await asyncio.gather(*self.tasks.values(), return_exceptions=True)
        if self.client:
            await self.client.aclose()

    def configured(self, name: str) -> bool:
        if name == "dx":
            return bool((self.store.get_setting("dx") or {}).get("token"))
        return bool((self.store.get_setting("da") or {}).get("access_token"))

    def restart(self, name: str) -> None:
        if old := self.tasks.pop(name, None):
            old.cancel()
        self.instances.pop(name, None)
        status = self.status[name]
        if not self.enabled or not self.configured(name):
            status.state, status.error = "off", None
            return
        if name == "dx":
            token = self.store.get_setting("dx")["token"]
            collector = DonateXCollector(self.store, token, self.hub.notify, self.client)
        else:
            collector = da.DACollector(self.store, self.rates, self.hub.notify, self.client)
        self.instances[name] = collector
        self.tasks[name] = asyncio.create_task(run_forever(name, collector.connect, status, lambda: None))

    def backfill(self, since: datetime) -> None:
        """Fetch donations back to `since` (e.g. a goal start moved into the past) without blocking the request."""
        async def one(name: str, collector) -> None:
            try:
                await collector.catch_up(since)
            except Exception as e:
                log.warning("%s backfill failed: %s", name, describe_error(e))

        for name, collector in list(self.instances.items()):
            task = asyncio.create_task(one(name, collector))
            self._backfills.add(task)
            task.add_done_callback(self._backfills.discard)

    def platforms(self) -> dict:
        da_cfg = self.store.get_setting("da") or {}
        out = {}
        for name, st in self.status.items():
            out[name] = {"configured": self.configured(name), "state": st.state, "error": st.error}
        out["da"]["has_client"] = bool(da_cfg.get("client_id") and da_cfg.get("client_secret"))
        out["da"]["client_id"] = da_cfg.get("client_id", "")
        out["da"]["redirect_uri"] = da.redirect_uri(self.port)
        return out


DA_DONE_PAGE = """<!doctype html><html lang="ru"><meta charset="utf-8"><title>DonationAlerts подключён</title>
<body style="font-family:system-ui,sans-serif;background:#1c1c1e;color:#ececee;display:grid;place-items:center;height:100vh;margin:0">
<div style="text-align:center"><h1 style="font-weight:600">DonationAlerts подключён</h1>
<p style="color:#9d9da4">Вкладку можно закрыть и вернуться в приложение.</p></div></body></html>"""


def create_app(store: Store, start_collectors: bool = True, port: int = DEFAULT_PORT) -> FastAPI:
    hub = Hub(store)
    collectors = Collectors(store, hub, port)
    oauth_states: set[str] = set()

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        if start_collectors:
            await collectors.start()
        yield
        await collectors.stop()

    app = FastAPI(lifespan=lifespan, docs_url=None, redoc_url=None)
    app.state.collectors = collectors
    app.state.hub = hub

    @app.middleware("http")
    async def same_origin_writes(request, call_next):
        # Any website open in the streamer's browser can POST to localhost; only our own pages may change state.
        origin = request.headers.get("origin")
        if request.method not in SAFE_METHODS and origin and urlsplit(origin).hostname not in LOCAL_HOSTS:
            return JSONResponse({"detail": "запрос с чужого сайта отклонён"}, status_code=403)
        return await call_next(request)

    # Blocks DNS rebinding: a foreign domain resolving to 127.0.0.1 still sends its own Host header.
    app.add_middleware(TrustedHostMiddleware, allowed_hosts=LOCAL_HOSTS)
    app.mount("/static", StaticFiles(directory=STATIC), name="static")

    def require_goal():
        goal = store.active_goal()
        if goal is None:
            raise HTTPException(404, "нет активной цели")
        return goal

    @app.get("/")
    async def admin_page():
        return FileResponse(STATIC / "admin.html")

    @app.get("/overlay")
    async def overlay_page():
        return FileResponse(STATIC / "overlay.html")

    @app.websocket("/ws")
    async def ws_endpoint(ws: WebSocket):
        await ws.accept()
        hub.register(ws)
        try:
            await ws.send_json(hub.snapshot())
            while True:
                await ws.receive_text()
        except WebSocketDisconnect:
            pass
        finally:
            hub.unregister(ws)

    @app.get("/api/state")
    async def state():
        return {**hub.snapshot(), "platforms": collectors.platforms(),
                "port": {"current": port, "saved": resolve_port(store)}}

    @app.put("/api/settings/port")
    async def set_port(body: PortSettings):
        store.set_setting("port", body.port)
        return {"current": port, "saved": resolve_port(store)}

    @app.get("/api/presets")
    async def presets():
        return load_presets()

    @app.put("/api/style")
    async def put_style(patch: Annotated[dict, Body()]):
        style = merge_style(hub.style(), patch)
        store.set_setting("style", style)
        await hub.broadcast()
        return style

    @app.post("/api/goal")
    async def create_goal(body: GoalCreate):
        try:
            goal = store.create_goal(body.title, body.target_rub, body.started_at or datetime.now(UTC), body.ends_at)
        except ValueError:
            raise HTTPException(409, "уже есть активная цель")
        collectors.backfill(goal.started_at)
        await hub.broadcast()
        return hub.snapshot()

    @app.patch("/api/goal")
    async def patch_goal(body: GoalPatch):
        goal = require_goal()
        fields = {k: getattr(body, k) for k in body.model_fields_set}
        if any(v is None for k, v in fields.items() if k != "ends_at"):
            raise HTTPException(422, "поле не может быть пустым")
        store.update_goal(goal.id, **fields)
        if "started_at" in fields:
            collectors.backfill(fields["started_at"])
        await hub.broadcast()
        return hub.snapshot()

    @app.post("/api/goal/close")
    async def close_goal():
        goal = require_goal()
        store.close_goal(goal.id, datetime.now(UTC))
        await hub.broadcast()
        return hub.snapshot()

    @app.get("/api/history")
    async def history():
        return [
            {"id": g.id, "title": g.title, "target_rub": g.target_rub, "collected_rub": round(total, 2),
             "started_at": to_iso(g.started_at), "closed_at": to_iso(g.closed_at)}
            for g, total in store.history()
        ]

    @app.post("/api/debug/donation")
    async def debug_donation(body: DebugDonation):
        hub.add_test_amount(body.amount_rub)
        await hub.broadcast()
        return hub.snapshot()

    @app.put("/api/platforms/dx")
    async def set_dx(body: DXSettings):
        store.set_setting("dx", {"token": body.token.strip()})
        collectors.restart("dx")
        return collectors.platforms()

    @app.put("/api/platforms/da")
    async def set_da(body: DASettings):
        cfg = store.get_setting("da") or {}
        if cfg.get("client_id") != body.client_id.strip():
            cfg = {}  # different app: old tokens are useless
        cfg.update(client_id=body.client_id.strip(), client_secret=body.client_secret.strip())
        store.set_setting("da", cfg)
        collectors.restart("da")
        return collectors.platforms()

    @app.get("/auth/da/start")
    async def da_start():
        cfg = store.get_setting("da") or {}
        if not cfg.get("client_id"):
            raise HTTPException(400, "сначала укажите client_id и client_secret")
        state = secrets.token_urlsafe(16)
        oauth_states.add(state)
        return RedirectResponse(da.authorize_url(cfg["client_id"], state, da.redirect_uri(port)))

    @app.get("/auth/da/callback")
    async def da_callback(state: str = "", code: str = "", error: str = ""):
        if state not in oauth_states:
            raise HTTPException(400, "неверный state, начните вход заново")
        oauth_states.discard(state)
        if error or not code:
            raise HTTPException(400, f"DonationAlerts отказал: {error or 'нет кода'}")
        cfg = store.get_setting("da") or {}
        async with httpx.AsyncClient() as client:
            try:
                cfg = await da.exchange_code(client, cfg, code, da.redirect_uri(port))
            except httpx.HTTPError as e:
                raise HTTPException(502, f"обмен кода не удался: {e}")
        store.set_setting("da", cfg)
        collectors.restart("da")
        # This page opens in the system browser, not in the app window.
        return HTMLResponse(DA_DONE_PAGE)

    return app
