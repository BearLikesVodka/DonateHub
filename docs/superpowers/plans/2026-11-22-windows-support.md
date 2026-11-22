# DonateHub на Windows и автообновление — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** DonateHub собирается и работает на Windows 10/11 так же, как на Linux, выпуски для обеих платформ собираются в GitHub Actions по тегу, программа сама предлагает обновления и ставит их по кнопке.

**Architecture:** Ядро (`crates/core`) получает платформенную папку данных, эксклюзивную привязку порта на Windows и точку подключения апдейтера (`updates::Updater`) с HTTP API `/api/update*`. Оболочка (`src-tauri`) делится на общий код и платформенные модули: трей ksni на Linux и встроенный трей Tauri на Windows. Апдейтер (`tauri-plugin-updater`) живёт в оболочке и регистрируется в ядре. Окно настроек показывает плашку обновления через HTTP API, IPC Tauri не используется. CI: тесты на Linux+Windows на каждый push, выпуск-черновик с `latest.json` по тегу `v*`.

**Tech Stack:** Rust stable (edition 2024), axum, Tauri 2 (+ updater, notification на Windows), ksni (Linux), windows-sys (консоль `--headless` на Windows), GitHub Actions (`tauri-apps/tauri-action`), NSIS, AppImage, node:test.

**Spec:** `docs/superpowers/specs/2026-11-22-windows-support-design.md`

## Global Constraints

- Только Windows 10/11 x86_64 и Linux x86_64. Linux-поведение (папка данных, порт, ksni, AppImage, окно) не меняется.
- Самые свежие версии крейтов и actions (`cargo add` без версии).
- Платформенный код — через `#[cfg(...)]` и target-зависимости в `Cargo.toml`. Linux-крейты (ksni, webkit2gtk, notify-rust, Linux-фичи arboard) не собираются под Windows, а фича `tray-icon` у tauri включается только для Windows.
- Сервер слушает только `127.0.0.1`. Новые эндпоинты подчиняются `trusted_host` и `same_origin_writes`.
- Ничего не ставится и не перезапускается без нажатия «Обновить». Обновления проверяются подписью всегда, в том числе при `DONATEHUB_UPDATE_ENDPOINT`.
- Тексты для пользователя — по-русски.
- Секреты (ключ подписи, пароль) никогда не печатаются в вывод, не коммитятся и не попадают в логи. Локально: `~/.tauri/donatehub.key`, `~/.tauri/donatehub.key.pub`, `~/.tauri/donatehub.key.password`, `chmod 600`.
- Коммиты только от `BearLikesVodka <33029026+BearLikesVodka@users.noreply.github.com>` (локальный git config уже настроен), trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Работа в ветке `feat/windows`, она отправляется на GitHub (`git push -u origin feat/windows`), чтобы CI проверял Windows. В `main` — только по команде пользователя.
- Перед каждым коммитом на Linux зелёные `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` и `node --test tests/js`. Начиная с Task 4 после push зелёный и Windows-job CI (`gh run watch`).
- Тестовые запуски на Linux — с временным `XDG_DATA_HOME`, портом ≠ 47321 и `APPIMAGELAUNCHER_DISABLE=1`. Живую базу пользователя не трогать.

## Review Focus

1. Второй экземпляр или чужая программа на Windows не может молча занять порт работающего DonateHub. Проверяется `second_server_on_same_port_fails` в CI на Windows (Task 2).
2. `/api/update/install` с чужим `Origin` → 403, с чужим `Host` → 400, без апдейтера → 404 с понятным текстом. Тесты `update_install_cross_site_rejected`, `update_unsupported_without_shell` (Task 3).
3. «Обновить» при отсутствии обновления или во время установки не запускает вторую установку. Тесты `install_requires_available_update`, `install_not_started_twice` (Task 6).
4. Уведомление о новой версии приходит один раз на версию, а не каждые 6 часов. Тест `notifies_once_per_version` (Task 6).
5. Тег, не совпадающий с версией в `Cargo.toml`, не собирает выпуск. Шаг проверки в `release.yml` и тег-проверка в Task 7.

---

### Task 1: CI на Linux и Windows

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: job `test` с матрицей `os: [ubuntu-24.04, windows-latest]`, запускается на `push` (любая ветка) и `pull_request`. Используется во всех следующих задачах.

- [ ] **Step 1: Создать ветку** `git switch -c feat/windows`.
- [ ] **Step 2: Написать `ci.yml`.** Шаги:
  1. checkout;
  2. Rust stable с clippy (`dtolnay/rust-toolchain@stable`), кэш (`Swatinem/rust-cache`);
  3. на Ubuntu системные пакеты Tauri: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libxdo-dev libssl-dev`;
  4. `actions/setup-node` (LTS);
  5. `cargo clippy --workspace --all-targets -- -D warnings`;
  6. `cargo test --workspace`;
  7. `node --test tests/js`.

  `fail-fast: false`, чтобы Linux-результат был виден даже при красном Windows.
- [ ] **Step 3: Push и проверка.** `git push -u origin feat/windows`, затем `gh run watch --exit-status`.
  Expected: Linux-job зелёный. Windows-job красный из-за ksni, webkit2gtk и `tokio::signal::unix`. Записать в ledger: `Task 1: Ruling: Windows CI red until Task 4 — expected`.
- [ ] **Step 4: Commit** `ci: test on Linux and Windows` (коммит уже отправлен в Step 3; при правках — новый коммит и push).

### Task 2: Ядро — папка данных Windows и эксклюзивный порт

**Files:**
- Modify: `crates/core/src/config.rs`, `crates/core/src/server.rs`
- Test: `crates/core/tests/config.rs`, `crates/core/tests/server.rs`

**Interfaces:**
- Produces: `pub fn windows_data_dir(env: &impl Fn(&str) -> Option<String>) -> std::io::Result<PathBuf>` — `%APPDATA%\DonateHub`, создаётся, если нет; ошибка, если `APPDATA` пуст. `data_dir(env)` на Windows вызывает её, на Linux работает как раньше. `pub const WINDOWS_APP_DIR: &str = "DonateHub"`. `port_free` и `Server::start` ставят `SO_REUSEADDR` только `#[cfg(not(windows))]`.

- [ ] **Step 1: Тесты (RED).**
  - `windows_data_dir_uses_appdata`: tempdir как `APPDATA` → `tmp/DonateHub`, `is_dir()`. Работает на любой ОС.
  - `windows_data_dir_requires_appdata`: пустой env → `Err`.
  - `second_server_on_same_port_fails` (в `tests/server.rs`): `Server::start` на свободном порту, второй `Server::start` на тот же порт → `Err`.
  - Существующие XDG/HOME-тесты `data_dir` пометить `#[cfg(not(windows))]`, добавить `#[cfg(windows)] data_dir_on_windows_uses_appdata`.
- [ ] **Step 2: Запуск** `cargo test -p donatehub-core --test config`.
  Expected: FAIL, `windows_data_dir` не найдена. `second_server_on_same_port_fails` на Linux проходит сразу: он сторожит Windows. Записать это ruling'ом.
- [ ] **Step 3: Реализация** в `config.rs` и `server.rs` по Interfaces.
- [ ] **Step 4: Запуск** полного набора проверок из Global Constraints. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): Windows data dir and exclusive port bind`, push.

### Task 3: Ядро — точка подключения апдейтера и API

**Files:**
- Create: `crates/core/src/updates.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/src/app.rs`
- Test: `crates/core/tests/app.rs`

**Interfaces:**
- Produces:
  ```rust
  // crates/core/src/updates.rs
  #[derive(Debug, Clone, PartialEq, serde::Serialize)]
  pub struct UpdateInfo { pub current: String, pub available: Option<String>, pub installing: bool }
  pub trait Updater: Send + Sync + 'static {
      fn info(&self) -> UpdateInfo;
      /// Start installing the available update in the background. Err(text for the user) if it cannot start.
      fn install(&self) -> Result<(), String>;
  }
  ```
  - `AppState::set_updater(&self, u: Arc<dyn Updater>)` хранит апдейтер в `Arc<OnceLock<Arc<dyn Updater>>>`, общем для клонов `AppState`; повторный вызов игнорируется.
  - `GET /api/update`: без апдейтера → `{"supported": false}`, с апдейтером → `{"supported": true, "current", "available", "installing"}`.
  - `POST /api/update/install`:
    - без апдейтера → 404 `{"detail": "обновление недоступно в этом режиме"}`;
    - `install()` вернул `Err(t)` → 409 `{"detail": t}`;
    - `Ok` → 202 `{"ok": true}`.

- [ ] **Step 1: Тесты (RED).** Фейковый апдейтер в тесте: `Mutex<UpdateInfo>` плюс счётчик вызовов `install`.
  - `update_unsupported_without_shell`: GET → `supported: false`; POST install → 404, `detail` — тот самый текст.
  - `update_info_from_shell`: после `set_updater` GET отдаёт поля фейка.
  - `update_install_calls_shell`: POST → 202, счётчик = 1.
  - `update_install_error_is_409`: фейк возвращает `Err("обновлений нет")` → 409 с этим `detail`.
  - `update_install_cross_site_rejected`: POST с `Origin: http://evil.example` → 403, счётчик = 0.
- [ ] **Step 2: Запуск** `cargo test -p donatehub-core --test app update_`. Expected: FAIL (нет модуля и маршрутов).
- [ ] **Step 3: Реализация.** Маршруты добавить в `router()` до слоёв middleware.
- [ ] **Step 4: Запуск** полного набора проверок. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): updater hook and /api/update`, push.

### Task 4: Оболочка — платформенное разделение (Windows-сборка зелёная)

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/src/main.rs`, `src-tauri/src/desktop.rs`
- Delete: `src-tauri/src/tray.rs`. Вместо него `src-tauri/src/tray/mod.rs` (общее), `tray/linux.rs` (ksni), `tray/windows.rs` (трей Tauri).
- Create: `src-tauri/src/notify.rs`, `src-tauri/src/clipboard.rs` (с cfg-ветками внутри)

**Interfaces:**
- Consumes: ничего нового из ядра.
- Produces:
  - `tray::spawn(app, port) -> bool` и `tray::on_hidden_to_tray(app)`, как сейчас;
  - `tray::{Action, MENU}` без изменений (тест `menu_labels` остаётся);
  - `tray::run_action(app, overlay_url, action)` — общий обработчик для обеих платформ;
  - `notify::show(body: String)`: Linux — notify-rust, Windows — `tauri-plugin-notification`. Плагин подключается только на Windows; на Linux оставить notify-rust, он проверен;
  - `clipboard::copy(text: String)`: Linux — `arboard` с `wait()` (как сейчас), Windows — `arboard` без `SetExtLinux`.

- [ ] **Step 1: `Cargo.toml`.**
  - `[target.'cfg(target_os = "linux")'.dependencies]`: `ksni`, `webkit2gtk`, `notify-rust`, `arboard` с `wayland-data-control`.
  - `[target.'cfg(windows)'.dependencies]`: `arboard`, `tauri` с `features = ["image-png", "tray-icon"]`, `tauri-plugin-notification`, `windows-sys` с `Win32_System_Console`.
- [ ] **Step 2: `desktop.rs`.**
  - Блок `with_webview` + webkit2gtk под `#[cfg(target_os = "linux")]`.
  - `quit_on_signals` под `#[cfg(unix)]`.
  - На Windows подключить `tauri_plugin_notification::init()`.
- [ ] **Step 3: `main.rs`.**
  - `#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]`.
  - В `run_headless` ожидание SIGTERM — `#[cfg(unix)]`, на Windows только Ctrl+C.
  - На Windows при `--headless` до первого вывода вызвать `AttachConsole(ATTACH_PARENT_PROCESS)`, чтобы адреса печатались в cmd/PowerShell.
- [ ] **Step 4: Трей Windows** (`tray/windows.rs`).
  - `TrayIconBuilder` с иконкой 32×32 (та же, что у ksni), подсказка `DonateHub`, меню из `MENU` с разделителем перед «Выход».
  - Левый клик (`MouseButton::Left`, `MouseButtonState::Up`) → `toggle_window`. Правый клик показывает меню (`show_menu_on_left_click(false)`).
  - `spawn` возвращает `true`, если трей создан.
- [ ] **Step 5: Проверка на Linux.** Полный набор проверок. Затем `cargo run -p donatehub` с временным `XDG_DATA_HOME` и `DONATEHUB_PORT=47390` в изолированной сессии (см. память `gui-test-isolation`): трей ksni регистрируется, «Выход» через dbusmenu завершает процесс.
  Expected: как до изменений.
- [ ] **Step 6: Commit** `feat(shell): split platform code, Windows tray and console`, push, `gh run watch --exit-status`.
  Expected: оба job'а зелёные. Если Windows-тесты падают на Linux-предположениях (пути, сигналы), чинить в этой задаче и записывать ruling.

### Task 5: Плашка обновления в окне настроек

**Files:**
- Modify: `static/render.js`, `static/admin.html`, `static/admin.js`, `static/admin.css`
- Test: `tests/js/render.test.mjs`

**Interfaces:**
- Consumes: `GET /api/update`, `POST /api/update/install` (Task 3).
- Produces: `export function updateBanner(info)` в `render.js`. Возвращает `null` или `{ text, button, busy }`:
  - `supported: false` или нет `available` → `null`;
  - `available: "0.3.1"`, не устанавливается → `{ text: "Доступна версия 0.3.1", button: "Обновить", busy: false }`;
  - `installing: true` → `{ text: "Устанавливается версия 0.3.1, программа перезапустится…", button: null, busy: true }`.

- [ ] **Step 1: Тест (RED)** `updateBanner covers states` с тремя случаями выше. `node --test tests/js` → FAIL (нет экспорта).
- [ ] **Step 2: Реализация.**
  - `render.js`: функция `updateBanner`.
  - `admin.html`: `<div id="update-banner" class="update-banner" hidden>` над содержимым `main`.
  - `admin.js`: `refreshUpdate()` вызывается в `init()` и раз в 10 минут. Кнопка делает `run(() => api("POST", "/api/update/install"))` и сразу перерисовывает плашку из `GET /api/update`.
  - Стиль — в духе текущих `.side-status` и тостов.
- [ ] **Step 3: Запуск** `node --test tests/js` → PASS. Ручная проверка в браузере с `--headless` и временным `XDG_DATA_HOME`: плашки нет (`supported: false`), ошибок в консоли нет.
- [ ] **Step 4: Commit** `feat(ui): update banner`, push.

### Task 6: Апдейтер в оболочке, ключи, конфиг установщика

**Files:**
- Create: `src-tauri/src/updates.rs`, `src-tauri/icons/icon.ico`
- Modify: `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/src/desktop.rs`, `packaging/build-appimage.sh`

**Interfaces:**
- Consumes: `donatehub_core::updates::{Updater, UpdateInfo}`, `AppState::set_updater` (Task 3), `notify::show` (Task 4).
- Produces:
  - `DesktopUpdater` (impl `Updater`) — регистрируется в `run()` после `build(...)`.
  - Чистая логика в `UpdateState` для юнит-тестов:
    - `fn set_available(&mut self, version: Option<String>) -> bool` — `true` ровно тогда, когда о версии ещё не уведомляли;
    - `fn begin_install(&mut self) -> Result<(), String>` — `Err("обновлений нет")`, если нет версии; `Err("обновление уже устанавливается")`, если уже идёт;
    - `fn supported(is_windows: bool, appimage_env: Option<&str>) -> bool` — Windows всегда, Linux только из AppImage (`APPIMAGE` не пуст).

- [ ] **Step 1: Тесты (RED)** в `src-tauri/src/updates.rs`:
  - `notifies_once_per_version`: `set_available(Some("0.3.1"))` → `true`, второй раз → `false`, `Some("0.3.2")` → `true`.
  - `install_requires_available_update`.
  - `install_not_started_twice`.
  - `supported_only_on_windows_or_appimage`.

  `cargo test -p donatehub updates` → FAIL.
- [ ] **Step 2: Логика `UpdateState`** — до PASS.
- [ ] **Step 3: Ключи подписи.**
  - Сгенерировать случайный пароль в `~/.tauri/donatehub.key.password`, затем `cargo tauri signer generate --ci -w ~/.tauri/donatehub.key -p "$(cat …password)"`. Выставить `chmod 600` на все три файла.
  - Секреты GitHub: `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/donatehub.key` и `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD < ~/.tauri/donatehub.key.password`.
  - Значения ни разу не выводить. Проверка: `gh secret list` показывает оба имени.
- [ ] **Step 4: `tauri.conf.json`.**
  - Удалить `"version"`: версия берётся из `Cargo.toml`, это единственный источник. Если Tauri не разрешает
    `version.workspace = true`, вернуть `"version"` в конфиг, а `release.yml` (Task 7) пусть сверяет тег с обоими местами.
    Это записать ruling'ом.
  - `bundle.targets: ["appimage", "nsis"]`, `bundle.createUpdaterArtifacts: true`.
  - `bundle.icon` + `icons/icon.ico`: 16–256 px из `static/icon.svg` через `rsvg-convert` и `magick`.
  - `bundle.windows.nsis`: `installMode: "currentUser"`, `languages: ["Russian"]`, `displayLanguageSelector: false`.
  - `plugins.updater`: `pubkey` — содержимое `~/.tauri/donatehub.key.pub`, `endpoints: ["https://github.com/BearLikesVodka/DonateHub/releases/latest/download/latest.json"]`, `windows.installMode: "passive"`.
- [ ] **Step 5: `DesktopUpdater`.**
  - Подключить `tauri-plugin-updater` (`.plugin(tauri_plugin_updater::Builder::new().build())`).
  - Фоновая задача: через 15 с после старта и затем раз в 6 ч `app.updater_builder()` (с `.endpoints([...])` при `DONATEHUB_UPDATE_ENDPOINT`) → `.build()?.check()`.
  - Найденный `Update` хранить. Если `set_available` вернул `true` → `notify::show("Доступна версия X. Обновить можно в окне настроек.")`. Ошибки проверки — только `tracing::warn!`.
  - `install()`: `begin_install`, затем в фоне `update.download(…)` → остановить сервер (как в `request_quit`, но без `exit`) → `update.install(bytes)` → `app.restart()`. При ошибке скачивания сбросить `installing` и записать в лог; сервер при этом не останавливается.
  - Если `!supported(...)`, апдейтер не регистрируется: `/api/update` → `supported: false`.
- [ ] **Step 6: Проверка.**
  - Полный набор проверок.
  - `./packaging/build-appimage.sh` с ключом в env (`TAURI_SIGNING_PRIVATE_KEY`, `…_PASSWORD` из файлов). Expected: рядом с AppImage появились `.AppImage.sig` или updater-архив.
  - Запуск AppImage (`APPIMAGELAUNCHER_DISABLE=1`, временный `XDG_DATA_HOME`, `DONATEHUB_PORT=47390`, изолированная сессия) → `GET /api/update` отдаёт `supported: true`, `current: "0.2.0"`, `available: null` (выпусков ещё нет, ошибка проверки только в логе).
- [ ] **Step 7: Commit** `feat(shell): auto-update via GitHub releases`, push, CI зелёный.

### Task 7: Выпуск 0.3.0 — workflow, README, тег

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `Cargo.toml` (версия `0.3.0`), `Cargo.lock`, `README.md`

**Interfaces:**
- Consumes: секреты Task 6, конфиг установщика Task 6.
- Produces: черновик выпуска `v0.3.0` с NSIS `.exe`, AppImage, их `.sig` и `latest.json`.

- [ ] **Step 1: `release.yml`.**
  - Триггер `push: tags: ["v*"]`.
  - Первый job сверяет тег с `version` из `[workspace.package]` в `Cargo.toml` и падает при несовпадении.
  - Матрица: `windows-latest` → `--bundles nsis`, `ubuntu-22.04` → `--bundles appimage` (старая glibc ради совместимости AppImage).
  - Шаги: Rust, кэш, системные пакеты на Ubuntu, `cargo install tauri-cli --locked`, затем `tauri-apps/tauri-action` с параметрами:
    - `tauriScript: cargo tauri`;
    - `tagName: ${{ github.ref_name }}`, `releaseName: DonateHub ${{ github.ref_name }}`;
    - `releaseDraft: true`, `includeUpdaterJson: true`;
    - `args` — из матрицы;
    - env: `GITHUB_TOKEN`, `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, на Linux `NO_STRIP: 1`.
  - `permissions: contents: write`.
- [ ] **Step 2: README.**
  - Раздел «Установка на Windows»: скачать `.exe` из последнего выпуска; SmartScreen → «Подробнее → Выполнить в любом случае»; данные в `%APPDATA%\DonateHub`; обновления через плашку в окне.
  - Раздел «Выпуск версии»: версия в `Cargo.toml` → тег `vX.Y.Z` → CI собирает черновик → проверка на Windows 11 и Windows 10 → «Publish release». Проверка обновления до публикации: скачать файлы черновика, поправить `latest.json` на локальные адреса, раздать локальным HTTP-сервером, запустить старую версию с `DONATEHUB_UPDATE_ENDPOINT`.
  - Пункт 1 дорожной карты отметить как «в проверке».
- [ ] **Step 3: Версия.** `0.3.0` в `[workspace.package]`, `cargo test --workspace` (обновит `Cargo.lock`), полный набор проверок, commit `release: 0.3.0`, push, CI зелёный.
- [ ] **Step 4: Тег.** `git tag v0.3.0 && git push origin v0.3.0`, затем `gh run watch --exit-status`.
  Expected: черновик выпуска, в нём `DonateHub_0.3.0_x64-setup.exe` (+`.sig`), `DonateHub_0.3.0_amd64.AppImage` (+`.sig`) и `latest.json`, где есть `windows-x86_64` и `linux-x86_64` с подписями. Проверка: `gh release view v0.3.0 --json assets,isDraft`.
- [ ] **Step 5: Передать пользователю чек-лист ручной проверки** из раздела «Проверка» спеки (пункты 1–5 на Windows 11 и Windows 10). Напомнить про резервную копию `~/.tauri/donatehub.key*`. Публикует выпуск и сливает `feat/windows` в `main` пользователь (или исполнитель по его команде).
