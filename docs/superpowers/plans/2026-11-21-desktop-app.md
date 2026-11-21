# Desktop App (AppImage) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Упаковать существующий сервер цели донатов в десктопное приложение (окно Qt WebEngine + трей) и собрать AppImage; порт по умолчанию 47321.

**Architecture:** Qt в главном потоке (окно `QWebEngineView` на локальную страницу настроек, `QSystemTrayIcon`, single-instance через `QLocalServer`), uvicorn в фоновом потоке. PyInstaller onedir → AppDir → appimagetool.

**Tech Stack:** Python 3.15, PySide6 6.12 (abi3: Essentials + Addons), PyInstaller 6.22, appimagetool.

**Spec:** `docs/superpowers/specs/2026-11-21-desktop-app-design.md` (+ базовый `2026-11-21-donation-goal-design.md`)

## Global Constraints

- `DEFAULT_PORT = 47321`; переменная `DONATION_GOAL_PORT` перекрывает настройку `port` в БД.
- Данные: `$XDG_DATA_HOME/donation-goal/app.db`, по умолчанию `~/.local/share/donation-goal/app.db`.
- Сервер слушает только `127.0.0.1`.
- Закрытие окна никогда не останавливает сервер; выход только из меню трея.
- Redirect URI DA: `http://127.0.0.1:<port>/auth/da/callback`.
- Токены не попадают в логи/URL оверлея (как раньше).

## Review Focus

1. Порт занят чужим процессом → понятное сообщение, а не падение в трейсбек. → `test_port_free_detects_busy_port` (Task 1) + диалог (Task 4).
2. Второй запуск → показывает первое окно, второй процесс выходит, сервер один. → `test_single_instance_forwards_show` (Task 4).
3. «Выход» → поток сервера завершается, порт освобождается. → `test_server_thread_stop_releases_port` (Task 3).
4. Смена порта меняет redirect URI DA и ссылку OBS; старые OAuth-токены DA при этом остаются валидны (redirect нужен только при входе). → `test_redirect_uri_follows_port` (Task 2).
5. OAuth-колбэк открыт в системном браузере → показывает страницу «Готово», а не админку в браузере. → `test_da_callback_success_page` (Task 2).

---

### Task 1: Config — data dir, port

**Files:** Create `src/donation_goal/config.py`; Test `tests/test_config.py`

**Interfaces — Produces:**
- `DEFAULT_PORT = 47321`
- `data_dir(env: Mapping[str, str] = os.environ) -> Path` — `$XDG_DATA_HOME/donation-goal` или `~/.local/share/donation-goal`; создаёт папку.
- `resolve_port(store: Store, env: Mapping[str, str] = os.environ) -> int` — env (валидный 1024–65535) > `store.get_setting("port")` > `DEFAULT_PORT`.
- `port_free(port: int) -> bool` — пробный `bind` на `127.0.0.1`.

- [ ] Тесты: `test_data_dir_uses_xdg`, `test_data_dir_default_home` (подмена `HOME`), `test_resolve_port_priority` (env > setting > default; мусор в env игнорируется), `test_port_free_detects_busy_port` (занять сокет → False; после закрытия → True).
- [ ] RED → реализация → GREEN → commit `feat: config for data dir and port`.

### Task 2: App — порт в приложении, настройка порта, страница после OAuth

**Files:** Modify `collectors/donationalerts.py`, `app.py`, `__main__.py`; Test `tests/test_donationalerts.py`, `tests/test_app.py`

**Interfaces:**
- `donationalerts.redirect_uri(port: int) -> str`; `authorize_url(client_id, state, redirect_uri)`; `exchange_code(client, cfg, code, redirect_uri)`. Константа `REDIRECT_URI` удаляется.
- `create_app(store, start_collectors=True, port=DEFAULT_PORT)`; `platforms()["da"]["redirect_uri"] = redirect_uri(port)`; `GET /api/state` дополнительно отдаёт `"port": {"current": port, "saved": resolve_port(store)}`.
- `PUT /api/settings/port {"port": int}` (1024–65535, иначе 422) → `store.set_setting("port", n)`.
- `GET /auth/da/callback` при успехе → HTML 200 «DonationAlerts подключён. Вкладку можно закрыть и вернуться в приложение.» (вместо редиректа на админку).
- `create_app` принимает `on_show: Callable[[], None] | None` — не нужно (YAGNI), не добавлять.

- [ ] Тесты: `test_redirect_uri_follows_port` (`create_app(..., port=50001)` → `redirect_uri` в `/api/state` содержит `:50001/`; `/auth/da/start` редирект содержит этот redirect_uri), `test_port_setting_validation` (70000 → 422; 48000 → 200 и `saved == 48000`), `test_da_callback_success_page` (подменить `da.exchange_code` через monkeypatch → 200, `text/html`, текст содержит «подключён»). Обновить `test_authorize_url` под новую сигнатуру.
- [ ] RED → реализация → GREEN → commit `feat: configurable port, DA redirect follows it`.

### Task 3: Server thread + запуск

**Files:** Create `src/donation_goal/server.py`; Modify `__main__.py`; Test `tests/test_server.py`

**Interfaces — Produces:**
- `class ServerThread(app: FastAPI, port: int)`: `start() -> None`, `wait_ready(timeout: float = 10) -> bool` (`server.started`), `stop(timeout: float = 10) -> None` (`should_exit = True`, `join`).
- `__main__.main(argv: list[str] | None = None)`: argparse `--headless`, `--hidden`. Общая инициализация: логирование (httpx → WARNING), `Store(data_dir() / "app.db")`, `port = resolve_port(store)`. `--headless` → `uvicorn.run(...)` как раньше; иначе `desktop.run(store, port, hidden=...)`.
- Точка входа `[project.scripts] donation-goal = "donation_goal.__main__:main"`.

- [ ] Тест `test_server_thread_stop_releases_port`: свободный порт → `start`, `wait_ready` True, `GET /api/state` 200 (httpx), `stop`, `port_free(port)` True.
- [ ] RED → реализация → GREEN → commit `feat: background server thread and launcher`.

### Task 4: Desktop shell

**Files:** Create `src/donation_goal/desktop.py`, `src/donation_goal/static/icon.svg`; Modify `pyproject.toml` (deps `PySide6-Essentials`, `PySide6-Addons`); Test `tests/test_desktop.py` (`QT_QPA_PLATFORM=offscreen`)

**Interfaces — Produces:**
- `instance_name() -> str` = `f"donation-goal-{os.getuid()}"`
- `notify_running_instance(name: str) -> bool` — подключиться `QLocalSocket`, отправить `b"show"`, True если получилось.
- `class SingleInstance(QObject)`: `__init__(name)`, сигнал `show_requested`; `listen()` (удаляет устаревший сокет через `QLocalServer.removeServer`).
- `class MainWindow(QMainWindow)`: `__init__(url: str)`; `closeEvent` → `hide()`, `event.ignore()`, сигнал `hidden_to_tray`. Внешние ссылки: свой `QWebEnginePage.acceptNavigationRequest` — если хост не `127.0.0.1`/`localhost` или путь `/auth/da/start`, то `QDesktopServices.openUrl` и False; `createWindow` (target=_blank) → то же.
  Настройка `JavascriptCanAccessClipboard = True`.
- `class Tray`: `QSystemTrayIcon` с меню «Открыть настройки», «Скопировать ссылку для OBS», «Выход»; клик (`Trigger`) → показать/скрыть окно; первое сворачивание → `showMessage`.
- `run(store: Store, port: int, hidden: bool) -> int`: QApplication (`setQuitOnLastWindowClosed(False)`, `setDesktopFileName("donation-goal")`), single-instance (если уже запущен → выход 0), `port_free` проверка → `QMessageBox` с полем нового порта (`QInputDialog.getInt`), сохранить и выйти с подсказкой перезапустить; `ServerThread.start/wait_ready`; окно (если не hidden); `app.exec()`; на выходе `server.stop()`.

- [ ] Тесты: `test_main_window_close_hides` (offscreen, `show()` → `close()` → `isVisible()` False, сигнал испущен), `test_single_instance_forwards_show` (`SingleInstance` слушает уникальное имя; `notify_running_instance` → True и сигнал `show_requested` получен через `QSignalSpy`/цикл событий; без сервера → False), `test_external_navigation_opens_browser` (monkeypatch `QDesktopServices.openUrl`; `page.acceptNavigationRequest(QUrl("https://www.donationalerts.com/x"), LinkClicked, True)` → False и openUrl вызван; `http://127.0.0.1:47321/#design` → True).
- [ ] RED → реализация → GREEN → commit `feat: desktop window and tray`.

### Task 5: Порт на странице «Подключения»

**Files:** Modify `static/admin.js`

- Карточка «Порт сервера»: число (текущее `saved`), кнопка «Сохранить порт» → `PUT /api/settings/port`; если `saved != current` — подсказка «Перезапустите приложение. Затем обновите ссылку в OBS и Redirect URL в приложении DonationAlerts».
- [ ] Проверка: браузерный скрипт (как `ui_check.py`) — смена порта показывает подсказку; консоль без ошибок. Commit `feat: port setting in UI`.

### Task 6: Сборка AppImage

**Files:** Create `packaging/donation-goal.spec`, `packaging/build-appimage.sh`, `packaging/AppRun`, `packaging/donation-goal.desktop`; Modify `pyproject.toml` (группа `build = ["pyinstaller"]`), `.gitignore` (`build/`, `dist/`), `README.md`

- spec: entry `src/donation_goal/__main__.py`, `datas` = `static/`, hiddenimports uvicorn (`uvicorn.logging`, `uvicorn.loops.auto`, `uvicorn.protocols.*`, `uvicorn.lifespan.on`), excludes ненужных Qt-модулей (`PySide6.Qt3D*`, `QtQuick3D`, `QtCharts`, `QtDataVisualization`, `QtMultimedia`, `QtPdf*` кроме нужного WebEngine, `QtBluetooth`, `QtSensors`, `QtSerialPort`, `QtTest`, `QtDesigner`); `name="donation-goal"`, onedir.
- build script: `uv sync --group build` → `pyinstaller` → AppDir (`usr/bin/donation-goal/…`, `AppRun`, `.desktop`, `donation-goal.svg`) → скачать `appimagetool-x86_64.AppImage` в `build/tools/` если нет → `ARCH=x86_64 appimagetool AppDir dist/DonationGoal-x86_64.AppImage`.
- [ ] Проверка: `./packaging/build-appimage.sh` успешно; `dist/DonationGoal-x86_64.AppImage --headless` отвечает на `:47321/api/state`; запуск без флагов: процесс жив, порт открыт, второй запуск завершается с кодом 0; скриншот окна. README: раздел «Запуск AppImage» вместо консольного. Commit `build: AppImage packaging`.
