# DonateHub на Rust (axum + Tauri) — дизайн

Переписывает Python-версию (`main`, тег `python-v0.1.0`) на Rust. Поведение для пользователя не меняется: всё, что описано
в `2026-11-21-donation-goal-design.md` и `2026-11-21-desktop-app-design.md`, остаётся в силе, кроме пунктов ниже.
Эталон поведения — Python-код в `src/donatehub/` и его тесты в `tests/`.

## Зачем

Python выбрали ради скорости написания MVP и сразу договорились потом перейти на Rust. Python-сборка — AppImage на
251 МБ (PyInstaller + Qt WebEngine/Chromium). Rust даёт один бинарник и лёгкий AppImage, а Tauri — нативное окно и
встроенный сборщик AppImage. Программа в первую очередь для самого стримера, на его машине (CachyOS, KDE Plasma, Wayland).

## Цели

- Функциональный паритет с Python-версией: коллекторы DA и DonateX, цели и история, стиль и пресеты, оверлей,
  страница настроек, окно + трей, один экземпляр, `--headless` / `--hidden`, окно «порт занят», тестовый донат на 15 секунд.
- Полная совместимость данных: та же папка `~/.local/share/donatehub/` (`$XDG_DATA_HOME/donatehub`), тот же `app.db`,
  та же схема и формат значений. Rust-версия, запущенная на базе Python-версии, показывает ту же цель, ту же историю,
  те же подключения — без повторного OAuth и без миграций.
- Фронтенд (`admin.*`, `overlay.*`, `render.js`, `presets/`, `icon.svg`) переиспользуется как есть (исключение — см. «Буфер обмена»); JSON API и
  WebSocket-сообщения совпадают с Python-версией байт-в-байт по структуре (ключи, типы, вложенность).
- `DonateHub-x86_64.AppImage` заметно меньше 251 МБ (ожидание ~80–100 МБ, не проверено).
- Клик по значку в трее показывает/прячет окно, как в Python-версии.

## Не цели

- macOS и Windows (будут «сильно позже»). Не закладывать абстракции под них; Linux-специфичное (трей `ksni`) держать
  в своём модуле, чтобы потом было что заменить.
- Новые функции, изменение интерфейса или API.
- Автозапуск, автообновление, Flatpak.
- Перевод фронтенда на Tauri IPC: страница остаётся обычной веб-страницей с сервера (её же грузит OBS).

## Анти-цели

- Rust-версия на живой базе пользователя меняет или портит данные так, что Python-версия (или следующий запуск)
  видит другую сумму. Особенно: другой формат дат в `donations.created_at` / `goals.*` ломает строковое сравнение в SQL.
- Двойной учёт или потеря донатов (дедуп по `(platform, external_id)`, догонка после переподключения).
- Токены в логах, в URL оверлея, в тексте ошибок на странице настроек.
- Закрытие окна останавливает сервер / оверлей в OBS.
- Второй запуск поднимает второй сервер.

## Ограничения

- Последний stable Rust (сейчас 1.99), edition 2024. Версии крейтов — самые свежие на момент начала работы (`cargo add`);
  Tauri 2.x.
- Сервер слушает только `127.0.0.1`, порт по умолчанию `47321`, приоритет: `DONATEHUB_PORT` (1024–65535) > настройка
  `port` в БД > по умолчанию.
- Перенос папки `~/.local/share/donation-goal` → `donatehub` повторить (у кого-то может остаться старая папка).
- Системные зависимости сборки уже есть на машине: webkit2gtk-4.1, GTK3. Tauri CLI нужно поставить (`cargo install tauri-cli`).

## Подходы

Решено с пользователем:

| Вопрос | Выбор | Отклонено |
|---|---|---|
| Что с Python-кодом | **A:** Rust заменяет Python в ветке `feat/rust-port`; Python-тесты — эталон до паритета, затем Python удаляется, на `main` перед слиянием ставится тег `python-v0.1.0` | B: Rust рядом в `rust/`, обе версии живут вместе — две сборки и дублирование |
| Трей | **A:** крейт `ksni` (StatusNotifierItem по D-Bus) — клик работает на KDE | B: встроенный трей Tauri — в Tauri 2 на Linux клик по значку не приходит (`TrayIconEvent`: «Linux: Unsupported»), только меню |
| Платформы | Только Linux | — |

Решено исполнителем (с причиной):

| Вопрос | Выбор | Почему не другое |
|---|---|---|
| Окно | Tauri-окно грузит `http://127.0.0.1:<port>/` с axum | Tauri IPC + свой протокол — пришлось бы менять фронтенд, а OBS всё равно нужен HTTP |
| Процесс | Один процесс: axum в tokio-рантайме Tauri | Отдельный сайдкар — две программы, синхронизация запуска/выхода |
| SQLite | `rusqlite` (feature `bundled`) под `Mutex`, синхронные запросы | `sqlx` — async и макросы ради нескольких запросов в минуту не окупаются |
| HTTP / WS-клиент | `reqwest` (rustls) / `tokio-tungstenite` (rustls) | — |
| SignalR (DonateX) | Свой минимальный клиент, как `collectors/signalr.py` | Готовых поддерживаемых Rust-клиентов SignalR нет |
| Время | `chrono` | `time` — подойдёт, но в форматировании дат Python-формата проще `chrono` |
| Логи | `tracing` + `tracing-subscriber` | — |
| Статика | Встраивается в бинарник (`rust-embed` или `include_dir`) | Раздача с диска — в AppImage пришлось бы таскать папку отдельно |

## Устройство

### Раскладка репозитория (cargo workspace в корне)

```
Cargo.toml                 # [workspace] members = ["crates/core", "src-tauri"]
crates/core/               # donatehub-core: всё без GUI — тестируется без GTK/WebKit
  src/config.rs            # data_dir (+ перенос donation-goal), resolve_port, port_free
  src/models.rs            # Donation, Goal, to_iso/from_iso
  src/store.rs             # SQLite, та же схема
  src/style.rs             # DEFAULT_STYLE, merge_style, пресеты
  src/currency.rs          # курс ЦБ
  src/hub.rs               # snapshot, рассылка в WS, временные тестовые суммы
  src/collectors/{base,donatex,donationalerts,signalr}.rs
  src/app.rs               # axum Router, middleware, эндпоинты, OAuth DA
  src/server.rs            # запуск/остановка сервера (graceful shutdown)
src-tauri/                 # donatehub: бинарник (Tauri), окно, трей (ksni), один экземпляр, CLI
static/                    # фронтенд (переезжает из src/donatehub/static/ на финальном шаге)
```

Пока Python не удалён, `crates/core` встраивает статику из `src/donatehub/static/`; при удалении Python папка
переезжает в `static/` (вместе с путём в `tests/js/render.test.mjs`).

### Совместимость данных (критично)

- Схема — ровно `SCHEMA` из `store.py` (`CREATE TABLE IF NOT EXISTS`), никаких новых таблиц и колонок.
- Даты в БД — строка `%Y-%m-%dT%H:%M:%S.%6f+00:00` в UTC (как `to_iso`), всегда 6 знаков долей секунды и `+00:00`.
  Чтение — любой ISO 8601 со смещением (как `datetime.fromisoformat`).
- `settings.value_json` — тот же JSON и те же ключи: `style`, `dx` (`{"token"}`), `da` (client_id, client_secret,
  токены, `expires_at` — формат смотреть в `donationalerts.py`), `port`, последний курс ЦБ (ключ — как в `currency.py`).
- `donations.raw_json` — исходный JSON доната от платформы.
- Сумма цели — тот же SQL, что `Store.collected`.

### Сервер (axum)

- Те же маршруты, методы, коды ответов и JSON: `/`, `/overlay`, `/static/*`, `WS /ws`, `/api/state`,
  `PUT /api/settings/port`, `/api/presets`, `PUT /api/style`, `POST|PATCH /api/goal`, `POST /api/goal/close`,
  `/api/history`, `POST /api/debug/donation`, `PUT /api/platforms/dx|da`, `/auth/da/start`, `/auth/da/callback`.
  Ошибки валидации — 422, как у FastAPI; фронтенду нужен текст ошибки — проверить, какое поле он читает (`detail`).
- Защита как в Python: запись (не GET/HEAD/OPTIONS) с `Origin` не с `127.0.0.1`/`localhost` → 403;
  `Host` не `127.0.0.1`/`localhost` (с портом или без) → 400 (защита от DNS rebinding).
- WS `/ws`: при подключении — snapshot, дальше рассылка при любом изменении; входящие сообщения игнорируются.
- Тестовый донат: сумма живёт только в памяти хаба и через 15 секунд снимается с новой рассылкой (как сейчас в `hub.py`).
- Выход: graceful shutdown axum, коллекторы останавливаются, порт освобождается.

### Коллекторы

Перенос `collectors/*.py` один в один: Centrifugo-протокол DA (connect → subscribe через `/api/v1/centrifuge/subscribe`),
OAuth DA с обновлением токена заранее, SignalR-клиент DonateX (negotiate → WS → handshake `\x1e`, типы 1/6/7),
догонка через REST после (пере)подключения, backoff 1 → 60 с со сбросом после стабильной работы, статусы
`off/connecting/ok/error` с текстами ошибок без токенов (`describe_error`, `redact`), курс ЦБ с суточным кэшем.
Падение одной платформы не трогает другую.

### Десктоп (Tauri + ksni)

- Окно Tauri 1280×860 на `http://127.0.0.1:<port>/`, открывается после готовности сервера. Окно сам грузит удалённый
  URL, Tauri IPC страницам не открывается (capabilities пустые для этого origin).
- Закрытие окна → `prevent_close` + `hide`; при первом скрытии — уведомление «Приложение работает в трее. Оверлей в OBS
  продолжает обновляться.» (через D-Bus `org.freedesktop.Notifications` — `notify-rust` или ksni; решение исполнителя).
- Трей `ksni`: иконка `icon.svg` (или PNG из неё), тултип `DonateHub`; левый клик (`activate`) — показать/скрыть окно;
  меню: «Открыть настройки», «Скопировать ссылку для OBS», «Выход». Действия трея передаются в Tauri через `AppHandle`
  (`run_on_main_thread`). Если трея нет (нет StatusNotifierWatcher) — закрытие окна сворачивает его, а не прячет.
- Навигация: переходы основного фрейма не на `127.0.0.1`/`localhost` и на `/auth/da/start` открываются в системном
  браузере (`tauri-plugin-opener` или `open`), окно остаётся на месте; `target=_blank` — так же.
- Буфер обмена: кнопки «Копировать» на странице должны работать (`navigator.clipboard` в WebKitGTK на `http://127.0.0.1`).
  Если WebKitGTK не даёт — разрешить через настройки webview (`with_webview` → WebKit settings) — решение исполнителя,
  но проверить обязательно. Если никакая настройка WebKitGTK не помогает, можно добавить в `admin.js` минимальный
  запасной путь (`document.execCommand('copy')` через скрытое поле). Это единственное разрешённое изменение фронтенда.
  «Скопировать ссылку для OBS» в трее — `arboard` или `tauri-plugin-clipboard-manager`.
- Один экземпляр: `tauri-plugin-single-instance`; второй запуск показывает окно первого и выходит с кодом 0.
- Порт занят чужим процессом (`port_free` до старта) → диалог (`rfd` / `tauri-plugin-dialog`) с вводом нового порта,
  сохранить в `settings.port`, сообщение «перезапустите приложение», выход.
- CLI: `--headless` (только сервер, без Tauri-окна; Ctrl+C/SIGTERM — корректный выход), `--hidden` (сразу в трей).
- Иконка, `.desktop` (`Name=DonateHub`, `StartupWMClass=donatehub`), имя бинарника `donatehub`.

### Сборка

- `cargo tauri build --bundles appimage` → переложить/переименовать в `dist/DonateHub-x86_64.AppImage`
  (скрипт `packaging/build-appimage.sh` переписывается под это).
- Python-упаковка (`packaging/*.spec`, `entry.py`, `AppRun`, `pyproject.toml`, `uv.lock`, `.python-version`) удаляется
  на финальном шаге вместе с `src/donatehub/` и `tests/*.py`.

## Порядок работы

1. Workspace, `crates/core`: config, models, store, style, currency — с тестами, перенесёнными из `tests/test_*.py`.
2. Hub + axum-приложение + server — тесты эндпоинтов как в `test_app.py` / `test_server.py`.
3. Коллекторы (SignalR, DonateX, DA) — тесты как в `test_signalr.py`, `test_donatex.py`, `test_donationalerts.py`, `test_base.py`.
4. Проверка паритета: обе версии на копии реальной базы (`XDG_DATA_HOME` во временной папке, разные порты) отдают
   одинаковый `/api/state` (без `platforms.*.state`) и `/api/history`. В копии перед запуском удаляются ключи
   `settings.da` и `settings.dx`, чтобы коллекторы не стартовали: иначе копия обновит токен DA (DA выдаёт новый
   refresh token, и старый в живой базе может перестать работать), а вторая версия откроет параллельное соединение
   с DonateX. Цели, донаты и стиль в копии остаются.
5. `src-tauri`: окно, трей ksni, один экземпляр, порт занят, CLI.
6. Сборка AppImage, ручная проверка. Всё, что можно проверить без экрана, исполнитель гоняет на изолированной папке
   данных и другом порту. Трей, уведомления, OBS и вход в DA проверяет сам пользователь. Вход DA возможен только
   на 47321, потому что этот Redirect URL прописан в приложении DA. Поэтому проверка делается при переключении:
   резервная копия `app.db` → «Выход» из Python-версии → запуск Rust-AppImage на живой базе и 47321.
   Откат простой: база общая, достаточно снова запустить Python-AppImage.
7. Тег `python-v0.1.0` на `main`, удаление Python, перенос `static/`, README, слияние в `main`.

## Что оставлено исполнителю

- Конкретные крейты там, где в таблице указано «или», точная раскладка модулей внутри `crates/core`.
- Как встроить статику и как отдать её с правильными MIME-типами.
- Механизм уведомлений и PNG для трея (ksni принимает pixmap или имя иконки).
- Структура Rust-тестов; где удобно — табличные тесты на тех же примерах JSON, что в Python-тестах.

## Проверка

- `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `node --test tests/js`.
- Паритет на копии реальной базы (шаг 4); исходная база пользователя не трогается до финальной проверки,
  и даже тогда — только после резервной копии.
- Ручная: AppImage стартует; окно открывается; закрытие → трей; клик по значку показывает/прячет окно; меню трея;
  второй запуск показывает первое окно; «Выход» освобождает порт; вход DA открывается в системном браузере;
  «Копировать» работает; тестовый донат появляется на 15 с; оверлей в OBS обновляется.
- При тестовых запусках AppImage — `APPIMAGELAUNCHER_DISABLE=1` или `--appimage-extract`, иначе AppImageLauncher
  перенесёт файл в `~/Applications`.
