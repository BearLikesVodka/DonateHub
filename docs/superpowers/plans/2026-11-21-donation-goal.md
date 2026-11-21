# Universal Donation Goal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Локальный сервер + OBS-оверлей цели донатов, суммирующий DonationAlerts и DonateX, со страницей настроек с живым превью и историей целей.

**Architecture:** Один процесс FastAPI на `127.0.0.1:8000`. Две asyncio-задачи-коллектора (DA Centrifugo, DonateX SignalR) пишут донаты в SQLite с дедупликацией; брокер рассылает снимок состояния по `WS /ws` оверлеям. Фронтенд — статические HTML/CSS/JS без сборки; превью = тот же оверлей в iframe.

**Tech Stack:** Python 3.15 (uv), FastAPI, uvicorn, httpx, websockets, sqlite3, pytest, pytest-asyncio; чистый JS (ES modules), `node --test` для JS-логики.

**Spec:** `docs/superpowers/specs/2026-11-21-donation-goal-design.md`

## Global Constraints

- `requires-python = ">=3.15"`; запуск: `uv run python -m donation_goal`.
- Сервер слушает только `127.0.0.1:8000`.
- Данные в `data/` (в `.gitignore`): `data/app.db`.
- Все суммы целей — RUB. Время в БД — UTC ISO 8601 (`YYYY-MM-DDTHH:MM:SS+00:00`).
- Токены никогда не отдаются в `/overlay`, `/ws` и в ответы `GET /api/*` (только флаги «настроено»).
- Тестовые донаты сохраняются, но не считаются.
- Размеры индикатора в `vw`.

## Review Focus

1. Один и тот же донат приходит дважды (realtime + догонка, `DonationCreated` + `DonationRerun`) — сумма не должна удвоиться. → тест `test_add_donation_dedup` (Task 1), `test_rerun_is_deduplicated` (Task 3).
2. Обрыв связи во время доната — после переподключения донат подтягивается догонкой. → `test_catchup_stops_at_boundary` (Tasks 3, 4).
3. Сумма > цели (как 33 825 / 5 000 в истории DA) — полоса 100%, текст реальный. → `test_percent_clamped` (Task 6).
4. DA-донат в USD/EUR при недоступном ЦБ — используется последний сохранённый курс; если курса нет вовсе, донат сохраняется с `amount_rub = NULL` и логируется, при следующем получении курса пересчитывается. → `test_convert_uses_cached_rate_when_offline` (Task 2).
5. DA `created_at` в формате `YYYY-MM-DD HH.MM.SS` и `YYYY-MM-DD HH:MM:SS` (UTC). → `test_parse_da_created_at_formats` (Task 4).

---

## File Structure

```
pyproject.toml
src/donation_goal/
  __main__.py          # uvicorn.run(app, host="127.0.0.1", port=8000)
  models.py            # Donation dataclass
  store.py             # SQLite: donations, goals, settings
  currency.py          # курс ЦБ, to_rub
  collectors/base.py   # CollectorStatus, run_forever (backoff)
  collectors/donatex.py
  collectors/signalr.py
  collectors/donationalerts.py
  hub.py               # Hub: снимок состояния + рассылка по WS
  style.py             # DEFAULT_STYLE, merge_style, load_presets
  app.py               # FastAPI: маршруты, lifespan
  static/overlay.html, overlay.css, overlay.js, render.js
  static/admin.html, admin.css, admin.js
  static/presets/*.json
tests/
  test_store.py, test_currency.py, test_donatex.py, test_signalr.py,
  test_donationalerts.py, test_app.py, js/render.test.mjs
```

---

### Task 1: Scaffold + Store

**Files:**
- Create: `pyproject.toml`, `src/donation_goal/__init__.py`, `src/donation_goal/models.py`, `src/donation_goal/store.py`
- Test: `tests/test_store.py`

**Interfaces:**
- Produces:
  - `@dataclass(frozen=True) Donation(platform: Literal["da","dx","manual"], external_id: str, username: str, amount: float, currency: str, amount_rub: float | None, created_at: datetime, is_test: bool, raw: dict)`
  - `@dataclass Goal(id: int, title: str, target_rub: float, started_at: datetime, ends_at: datetime | None, closed_at: datetime | None, manual_adjust_rub: float)`
  - `Store(path: str | Path)` (`":memory:"` допустим):
    - `add_donation(d: Donation) -> bool` — True, если новый (`INSERT OR IGNORE` по `(platform, external_id)`).
    - `set_amount_rub(platform, external_id, amount_rub) -> None`; `donations_missing_rub() -> list[Donation]`
    - `latest_donation_time(platform) -> datetime | None`
    - `active_goal() -> Goal | None`; `create_goal(title, target_rub, started_at, ends_at) -> Goal` (ValueError, если активная уже есть); `update_goal(goal_id, **fields) -> Goal`; `close_goal(goal_id, closed_at) -> Goal`
    - `collected(goal: Goal) -> float` — `SUM(amount_rub)` где `is_test=0`, `amount_rub IS NOT NULL`, `started_at <= created_at < COALESCE(closed_at, ends_at, +inf)` (если оба заданы — минимальный) + `manual_adjust_rub`.
    - `history() -> list[tuple[Goal, float]]` — закрытые цели, новые сверху.
    - `get_setting(key, default=None) -> Any`; `set_setting(key, value) -> None` (JSON).

- [ ] **Step 1: pyproject** — пакет `donation_goal` в `src/`, deps `fastapi`, `uvicorn[standard]`, `httpx`, `websockets`; dev `pytest`, `pytest-asyncio` (`asyncio_mode = "auto"`). `uv sync` проходит.
- [ ] **Step 2: Тесты**

```python
def test_add_donation_dedup(store):
    d = make_donation("dx", "a1", rub=100)
    assert store.add_donation(d) is True
    assert store.add_donation(d) is False

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
```
(`t(h)` — фикстура: `datetime(2026,11,21,h,tzinfo=UTC)`; `make_donation` — хелпер в `tests/conftest.py`.)

- [ ] **Step 3:** `uv run pytest tests/test_store.py` → FAIL (нет модуля).
- [ ] **Step 4:** Реализовать `models.py`, `store.py`. Datetime хранить как ISO-строки UTC (сравнение строк корректно при едином формате). Соединение `check_same_thread=False`, вызовы из одного event loop.
- [ ] **Step 5:** `uv run pytest tests/test_store.py` → PASS.
- [ ] **Step 6:** Commit `feat: storage for donations, goals, settings`.

---

### Task 2: Currency

**Files:** Create `src/donation_goal/currency.py`; Test `tests/test_currency.py`

**Interfaces:**
- Consumes: `Store.get_setting/set_setting` (ключ `"cbr_rates"`: `{"fetched_at": iso, "rates": {"USD": 92.1, ...}}`, где rate = RUB за 1 единицу).
- Produces: `class RateProvider(store: Store, client: httpx.AsyncClient, url: str = "https://www.cbr-xml-daily.ru/daily_json.js")`:
  - `async to_rub(amount: float, currency: str) -> float | None` — RUB → без запроса; кэш младше 24 ч → без запроса; иначе запрос, при ошибке — старый кэш; нет курса → None.
  - Курс ЦБ: `Valute[CODE].Value / Valute[CODE].Nominal`.

- [ ] **Step 1: Тесты** (httpx `MockTransport`):
  - `test_rub_passthrough`: `to_rub(150, "RUB") == 150`, ноль HTTP-запросов.
  - `test_convert_fetches_and_caches`: ответ `{"Valute": {"USD": {"Value": 90.0, "Nominal": 1}, "KZT": {"Value": 18.0, "Nominal": 100}}}` → `to_rub(2,"USD")==180`, `to_rub(1000,"KZT")==180`, всего 1 запрос.
  - `test_convert_uses_cached_rate_when_offline`: кэш 3-дневной давности с USD=80, транспорт кидает `httpx.ConnectError` → `to_rub(1,"USD")==80`.
  - `test_unknown_currency_returns_none`: пустой кэш + ошибка сети → None.
- [ ] **Step 2:** запуск → FAIL. **Step 3:** реализация. **Step 4:** → PASS. **Step 5:** Commit `feat: CBR currency conversion`.

---

### Task 3: DonateX collector

**Files:** Create `collectors/base.py`, `collectors/signalr.py`, `collectors/donatex.py`; Test `tests/test_signalr.py`, `tests/test_donatex.py`

**Interfaces:**
- Consumes: `Store`, `Donation`.
- Produces:
  - `base.py`: `@dataclass CollectorStatus(state: Literal["off","connecting","ok","error"]="off", error: str | None = None)`; `async run_forever(name, connect: Callable[[], Awaitable[None]], status: CollectorStatus, on_status: Callable[[], None])` — бесконечный цикл, при исключении `state="error"`, backoff 1,2,4…60 с, сброс backoff после успешной работы >60 с.
  - `signalr.py`: `encode(msg: dict) -> str` (JSON + `"\x1e"`), `decode(buf: str) -> list[dict]` (split по `\x1e`, пустые пропускать); `async signalr_stream(hub_url: str, token: str) -> AsyncIterator[tuple[str, list]]` — `POST {hub}/negotiate?negotiateVersion=1&access_token=…`, WS на `wss://…{hub}?id={connectionToken}&access_token=…`, handshake `{"protocol":"json","version":1}`, отвечает на ping (type 6) ping'ом, yield `(target, arguments)` для type 1, type 7 → исключение `ConnectionError(error)`.
  - `donatex.py`: `parse_dx_donation(obj: dict) -> Donation` (platform `"dx"`, `external_id=id`, `amount_rub=amountInRub`, `created_at=timestamp`, `is_test=isTest`); `class DonateXCollector(store, token: str, on_new: Callable[[], None], client: httpx.AsyncClient)` c `async catch_up(since: datetime) -> int` и `async connect() -> None` (catch_up → stream; `DonationCreated`/`DonationRerun` → `add_donation`, при True → `on_new()`). `since` = `min(начало активной цели, latest_donation_time("dx"))`, если ничего нет — пропуск.

- [ ] **Step 1: Тесты**
  - `test_decode_multiple_frames`: `decode('{"type":6}\x1e{"type":1,"target":"X","arguments":[1]}\x1e')` → 2 словаря.
  - `test_parse_dx_donation`: пример из документации (`amount: 100, currency: "USD", amountInRub: 9200, isTest: false, timestamp: "2026-10-12T18:42:07Z"`) → поля Donation; `created_at.tzinfo` = UTC.
  - `test_rerun_is_deduplicated`: прогнать через обработчик событий `DonationCreated` и `DonationRerun` с одним `id` → в store одна запись, `on_new` вызван 1 раз.
  - `test_catchup_stops_at_boundary`: MockTransport отдаёт страницы `take=100` (первая — 100 донатов, последний старше `since`) → запрошена только 1 страница, сохранены только донаты новее `since`; в запросе `sortOrder=NewestFirst`, заголовок `Authorization: Bearer tok`.
- [ ] **Step 2:** → FAIL. **Step 3:** реализация (обработчик события вынести в `handle_event(target, args) -> None` для теста). **Step 4:** → PASS.
- [ ] **Step 5:** Commit `feat: DonateX collector over SignalR`.

---

### Task 4: DonationAlerts OAuth + collector

**Files:** Create `collectors/donationalerts.py`; Test `tests/test_donationalerts.py`

**Interfaces:**
- Consumes: `Store` (ключ `"da"`: `{"client_id","client_secret","access_token","refresh_token","expires_at"}`), `RateProvider`, `run_forever`.
- Produces:
  - `REDIRECT_URI = "http://127.0.0.1:8000/auth/da/callback"`; `SCOPES = "oauth-user-show oauth-donation-subscribe oauth-donation-index"`
  - `authorize_url(client_id: str, state: str) -> str` (`https://www.donationalerts.com/oauth/authorize`, `response_type=code`)
  - `async exchange_code(client, cfg, code) -> dict`; `async refresh(client, cfg) -> dict` (`POST /oauth/token`, form; если нового refresh_token нет — сохранить старый); `expires_at = now + expires_in`.
  - `parse_created_at(s: str) -> datetime` (оба формата, UTC); `parse_da_donation(obj: dict, amount_rub: float | None) -> Donation` (platform `"da"`, `external_id=str(id)`, `is_test=False`)
  - `class DACollector(store, rates, on_new, client)`: `async ensure_token() -> str` (refresh, если до истечения < 1 сут.), `async catch_up(since) -> int` (`GET /api/v1/alerts/donations?page=N`, остановка, когда донат старше `since` или нет `links.next`), `async connect()` — `GET /api/v1/user/oauth` → WS `wss://centrifugo.donationalerts.com/connection/websocket` → `{"params":{"token":…},"id":1}` → `client` → `POST /api/v1/centrifuge/subscribe` `{"channels":["$alerts:donation_<id>"],"client":…}` → `{"params":{"channel":…,"token":…},"method":1,"id":2}` → читать сообщения; донат искать функцией `extract_push_donation(msg: dict) -> dict | None` (проверять `result.data.data`, затем `result.data`; объект доната — dict с ключами `id` и `amount`); незнакомое — `log.warning` с сообщением целиком.
  - Сообщения Centrifugo могут прийти несколькими JSON через `\n` в одном фрейме — разбирать построчно.
  - `async fix_missing_rub() -> int` — для `store.donations_missing_rub()` пересчитать через `rates`, вызывается при каждом connect и раз в час; при изменении → `on_new()`.
  - Тест `test_fix_missing_rub`: донат USD с `amount_rub=None`, `rates` теперь возвращает 90 → `amount_rub == 90`, `on_new` вызван.

- [ ] **Step 1: Тесты**
  - `test_parse_da_created_at_formats`: `"2026-11-21 09.05.07"` и `"2026-11-21 09:05:07"` → `datetime(2026,11,21,9,5,7,tzinfo=UTC)`.
  - `test_extract_push_donation`: `{"result":{"channel":"$alerts:donation_3","data":{"data":{"id":5,"amount":100,...}}}}` → dict с id 5; `{"result":{"type":1,...,"data":{"info":{}}}}` → None.
  - `test_authorize_url`: содержит `client_id=7`, `response_type=code`, `scope=oauth-user-show+oauth-donation-subscribe+oauth-donation-index` (или `%20`), закодированный `REDIRECT_URI`.
  - `test_refresh_keeps_old_refresh_token`: ответ без `refresh_token` → в store старый.
  - `test_catchup_stops_at_boundary`: 2 страницы по 30, граница на первой → 1 запрос; USD-донат получает `amount_rub` через подставной `rates`.
- [ ] **Step 2:** → FAIL. **Step 3:** реализация. **Step 4:** → PASS.
- [ ] **Step 5:** Commit `feat: DonationAlerts OAuth and Centrifugo collector`.

---

### Task 5: Style, Hub, App

**Files:** Create `style.py`, `hub.py`, `app.py`, `__main__.py`, `static/presets/*.json` (5 пресетов: «Зелёный DA», «Оранжевый», «Розовый», «Бирюзовый стекло», «Красный металл»); Test `tests/test_app.py`

**Interfaces:**
- Consumes: всё выше.
- Produces:
  - `style.py`: `DEFAULT_STYLE: dict` со схемой ниже; `merge_style(base: dict, patch: dict) -> dict` (рекурсивно, неизвестные ключи отбрасываются, типы проверяются по DEFAULT_STYLE); `load_presets() -> list[dict]` (`{"name", "style"}`).

    ```
    elements: {title_pos: "top"|"bottom"|"hidden", progress_pos: "inside"|"top"|"bottom"|"hidden",
               progress_format: "percent"|"amount"|"amount_percent"|"amount_target"|"amount_target_percent",
               time_pos: "top"|"bottom"|"hidden", show_bounds: bool, backdrop: bool, backdrop_color: str}
    bar: {height_vw: 7, radius_vw: 1.2, border_vw: 0, border_color: str,
          bg: {mode: "color"|"gradient", color1, color2, angle}, fill: {mode, color1, color2, angle}}
    fonts: {title|progress|time: {family: str, size_vw: float, weight: int, italic: bool,
            color: str, stroke_vw: float, stroke_color: str, shadow: bool}}
    ```
    Значения по умолчанию — как на скриншоте DA (заголовок сверху, прогресс внутри, `amount_target_percent`, время снизу, фон `rgba(43,43,43,0.91)`, заливка градиент `#009e05 → #00de07` 0°, шрифт `Fira Sans Condensed` 800 белый).
  - `hub.py`: `class Hub(store)`: `snapshot() -> dict` = `{"goal": {id,title,target_rub,started_at,ends_at} | None, "collected_rub": float, "style": dict}`; `register(ws)`, `unregister(ws)`, `async broadcast() -> None` (отправить снимок всем, мёртвые сокеты убрать); `notify()` — `asyncio.create_task(broadcast())` для синхронных колбэков.
  - `app.py`: `create_app(store, start_collectors: bool = True) -> FastAPI`. Маршруты:
    - `GET /` → `admin.html`; `GET /overlay` → `overlay.html`; `/static/*`.
    - `WS /ws` → сразу снимок, далее рассылки.
    - `GET /api/state` → снимок + `{"platforms": {"da": {configured, state, error}, "dx": {...}}}`.
    - `PUT /api/style` (body: style) → merge, сохранить, broadcast. `GET /api/presets`.
    - `POST /api/goal` `{title, target_rub, started_at?, ends_at?}` (started_at по умолчанию — сейчас); `PATCH /api/goal` (поля + `manual_adjust_rub`); `POST /api/goal/close`; `GET /api/history` → `[{title,target_rub,collected_rub,started_at,closed_at}]`.
    - `PUT /api/platforms/dx` `{token}`; `PUT /api/platforms/da` `{client_id, client_secret}`; `GET /auth/da/start` → redirect на `authorize_url` (state в памяти); `GET /auth/da/callback?code&state` → обмен, redirect `/#connections`. Изменение настроек платформы перезапускает её задачу.
    - `POST /api/debug/donation` `{amount_rub, username?}` → `Donation(platform="manual", external_id=uuid4)` — для проверки без реальных донатов.
  - `__main__.py`: `Store("data/app.db")` (создать папку), `uvicorn.run(create_app(store), host="127.0.0.1", port=8000)`.

- [ ] **Step 1: Тесты** (`TestClient`, `create_app(Store(":memory:"), start_collectors=False)`):
  - `test_ws_receives_update_after_debug_donation`: создать цель 10000 → открыть `/ws` (первое сообщение `collected_rub==0`) → `POST /api/debug/donation {"amount_rub": 500}` → следующее сообщение `collected_rub==500`.
  - `test_style_merge_rejects_unknown_and_bad_types`: `PUT /api/style {"bar":{"height_vw":"x","nope":1}}` → `height_vw` остался 7, `nope` отсутствует.
  - `test_tokens_not_exposed`: после `PUT /api/platforms/dx {"token":"SECRET"}` ни `/api/state`, ни сообщение `/ws` не содержат `"SECRET"`; `platforms.dx.configured is True`.
  - `test_goal_lifecycle`: create → второй create → 409 → close → `/api/history` длины 1.
- [ ] **Step 2:** → FAIL. **Step 3:** реализация. **Step 4:** → PASS.
- [ ] **Step 5:** Commit `feat: web app, hub and widget style`.

---

### Task 6: Overlay

**Files:** Create `static/render.js`, `static/overlay.html`, `static/overlay.css`, `static/overlay.js`; Test `tests/js/render.test.mjs`

**Interfaces:**
- Consumes: снимок `{goal, collected_rub, style}` из Task 5.
- Produces (`render.js`, ES module):
  - `formatAmount(n: number) -> string` — целые с пробелом-разделителем тысяч (`5 000`), дробная часть отбрасывается.
  - `formatProgress(collected, target, format) -> string` → `50%`, `5 000 RUB`, `5 000 RUB (50%)`, `5 000 / 10 000 RUB`, `5 000 / 10 000 RUB (50%)`; процент — целый, округление вниз, без ограничения 100.
  - `fillPercent(collected, target) -> number` — 0..100.
  - `formatRemaining(endsAt: Date | null, now: Date) -> string` — `""` без даты, `"Сбор завершён"` после, иначе `"Осталось 2 д 04:12:33"` / `"Осталось 04:12:33"`.
  - `render(root: HTMLElement, snapshot) -> void` — строит DOM виджета, стиль через CSS-переменные на root (`--bar-h`, `--bar-r`, `--fill-bg`, …), Google-шрифт подгружается добавлением `<link>` (если семейство в списке `GOOGLE_FONTS`, иначе используется как локальное).
- `overlay.js`: подключение к `ws://{location.host}/ws` с переподключением (1 с → 10 с); при `?preview=1` не подключается к WS, а слушает `postMessage({type:"preview", snapshot})`; ежесекундно обновляет оставшееся время. Без цели — пустая прозрачная страница. `body` прозрачный. Анимация заливки `transition: width .8s ease`.

- [ ] **Step 1: Тесты** (`node --test tests/js`):
  - `formatProgress(5000, 10000, "amount_target_percent") === "5 000 / 10 000 RUB (50%)"` и остальные четыре формата.
  - `test_percent_clamped`: `fillPercent(33825.61, 5000) === 100`, `formatProgress(33825.61, 5000, "percent") === "676%"`.
  - `formatRemaining(new Date("2026-11-23T04:12:33Z"), new Date("2026-11-21T00:00:00Z")) === "Осталось 2 д 04:12:33"`; `formatRemaining(null, now) === ""`.
- [ ] **Step 2:** → FAIL. **Step 3:** реализация. **Step 4:** → PASS.
- [ ] **Step 5:** Ручная проверка: `uv run python -m donation_goal`, создать цель через `curl -X POST /api/goal`, открыть `/overlay` в Playwright, скриншот; затем `POST /api/debug/donation` → полоса растёт без перезагрузки.
- [ ] **Step 6:** Commit `feat: OBS overlay`.

---

### Task 7: Admin page

**Files:** Create `static/admin.html`, `static/admin.css`, `static/admin.js`

**Interfaces:**
- Consumes: API из Task 5, `render.js` не нужен напрямую — превью = `<iframe src="/overlay?preview=1">`.
- Produces: вкладки по `location.hash`: `#goal`, `#design`, `#history`, `#connections`.
  - **Цель:** форма создания (название, сумма, старт — `datetime-local`, по умолчанию сейчас; окончание — опционально); у активной: собрано / цель, правка полей, ручная корректировка (±), кнопка «Завершить сбор» с подтверждением.
  - **Дизайн:** слева превью (16:9, переключатель «Игровой фон» — фоновое изображение-градиент из CSS, без внешних картинок), справа пресеты (клик → применить в форму), ниже подвкладки «Элементы» / «Индикатор» / «Шрифты» с полями из схемы стиля (селекты, ползунок+число, `<input type=color>` + текст для rgba, переключатель цвет/градиент). Любое изменение → `iframe.contentWindow.postMessage({type:"preview", snapshot})` с текущей целью (или `5000/10000 «Тестовая цель»`, если цели нет). «Сохранить изменения» → `PUT /api/style`; несохранённые изменения помечаются.
  - **История:** таблица «Цель сбора / Собрано / Начало / Окончание», сумма выше цели — оранжевым.
  - **Подключения:** DonateX — поле токена (type=password) + сохранить; DA — пошаговая инструкция (зарегистрировать приложение на `https://www.donationalerts.com/application/clients`, Redirect URI `http://127.0.0.1:8000/auth/da/callback`), поля client_id/secret, кнопка «Войти через DonationAlerts» → `/auth/da/start`; статусы платформ (обновление раз в 5 с из `/api/state`); кнопка «Тестовый донат 100 ₽» → `/api/debug/donation`. Блок «Ссылка для OBS» с `http://127.0.0.1:8000/overlay` и рекомендацией размера источника (например 800×200).
  - Тёмная тема с оранжевым акцентом, как у DA.

- [ ] **Step 1:** Реализация.
- [ ] **Step 2:** Проверка в Playwright: каждая вкладка без ошибок в консоли; смена высоты/цвета меняет превью до сохранения; после «Сохранить» открытый `/overlay` в другой вкладке обновился; создание и закрытие цели появляется в истории. Скриншоты вкладок.
- [ ] **Step 3:** Commit `feat: settings page with live preview`.

---

### Task 8: README и финальная проверка

**Files:** Create `README.md`

- [ ] **Step 1:** README на русском: установка (`uv sync`), запуск, подключение DA и DonateX, добавление в OBS (браузерный источник, URL, размер, «Обновлять браузер при активации сцены» не нужно).
- [ ] **Step 2:** `uv run pytest` и `node --test tests/js` → всё PASS; `uv run python -m donation_goal` стартует без настроенных платформ (статусы «не настроено», без падений).
- [ ] **Step 3:** Commit `docs: README`.
