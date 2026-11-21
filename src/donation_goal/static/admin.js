import { GOOGLE_FONTS, formatAmount, formatProgress, fillPercent } from "./render.js";

const $ = (sel, root = document) => root.querySelector(sel);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const clone = (o) => structuredClone(o);

const SAMPLE_GOAL = { title: "Тестовая цель", target_rub: 10000, ends_at: null };
const SAMPLE_COLLECTED = 5000;

const state = {
  snapshot: null,      // last snapshot from /ws
  platforms: null,     // from /api/state
  savedStyle: null,    // style as stored on the server
  style: null,         // style being edited
  presets: [],
  sub: "elements",
};

// ---------- api ----------

async function api(method, url, body) {
  const resp = await fetch(url, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await resp.json().catch(() => null);
  if (!resp.ok) {
    const detail = data?.detail;
    const msg = typeof detail === "string" ? detail : Array.isArray(detail) ? "Проверьте заполнение полей" : `Ошибка ${resp.status}`;
    throw new Error(msg);
  }
  return data;
}

let toastTimer;
function toast(text, bad = false) {
  const t = $("#toast");
  t.textContent = text;
  t.classList.toggle("bad", bad);
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), 3200);
}

async function run(fn, okText) {
  try {
    await fn();
    if (okText) toast(okText);
  } catch (e) {
    toast(e.message, true);
  }
}

// ---------- dates ----------

function toLocalInput(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  const off = d.getTimezoneOffset() * 60000;
  return new Date(d.getTime() - off).toISOString().slice(0, 16);
}

const fromLocalInput = (v) => (v ? new Date(v).toISOString() : null);

const fmtDate = (iso) => (iso ? new Date(iso).toLocaleString("ru-RU", { day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit" }) : "—");

// ---------- navigation ----------

const TABS = ["goal", "design", "history", "connections"];

function showTab() {
  const tab = TABS.includes(location.hash.slice(1)) ? location.hash.slice(1) : "goal";
  for (const t of TABS) $(`#tab-${t}`).hidden = t !== tab;
  for (const a of document.querySelectorAll("nav a")) {
    if (a.getAttribute("href") === `#${tab}`) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  }
  if (tab === "history") renderHistory();
  if (tab === "connections") refreshPlatforms();
}

// ---------- goal tab ----------

function renderGoal() {
  const view = $("#goal-view");
  const snap = state.snapshot;
  if (!snap) return;
  const g = snap.goal;
  if (!g) {
    if ($("#create-goal")) return;
    view.innerHTML = `
      <div class="card">
        <h2>Новая цель</h2>
        <p class="hint">Сюда идут донаты с DonationAlerts и DonateX, начиная с даты старта.</p>
        <form class="form" id="create-goal">
          <div class="row"><label for="g-title">Название</label><input id="g-title" type="text" required maxlength="200" placeholder="На новый микрофон"></div>
          <div class="row"><label for="g-target">Сумма, ₽</label><input id="g-target" type="number" min="1" step="1" required placeholder="10000"></div>
          <div class="row"><label for="g-start">Старт</label><input id="g-start" type="datetime-local" value="${toLocalInput(new Date().toISOString())}"></div>
          <div class="row"><label for="g-end">Окончание</label><input id="g-end" type="datetime-local"></div>
          <div class="row"><span></span><div class="actions"><button class="btn primary">Начать сбор</button></div></div>
        </form>
      </div>`;
    $("#create-goal").addEventListener("submit", (e) => {
      e.preventDefault();
      run(async () => {
        await api("POST", "/api/goal", {
          title: $("#g-title").value.trim(),
          target_rub: Number($("#g-target").value),
          started_at: fromLocalInput($("#g-start").value) ?? undefined,
          ends_at: fromLocalInput($("#g-end").value) ?? undefined,
        });
      }, "Сбор начат");
    });
    return;
  }

  // Don't wipe a form the streamer is typing into when a donation arrives: update only the numbers.
  const existing = $("#goal-live");
  if (existing && Number(existing.dataset.goalId) === g.id) {
    updateGoalNumbers(g, snap.collected_rub);
    return;
  }

  view.innerHTML = `
    <div class="card" id="goal-live" data-goal-id="${g.id}">
      <div class="goal-head">
        <div class="goal-title" id="gl-title"></div>
        <div class="goal-sum" id="gl-sum"></div>
      </div>
      <div class="goal-meter"><span id="gl-meter"></span></div>
      <div class="goal-meta" id="gl-meta"></div>
    </div>

    <div class="card">
      <h2>Изменить цель</h2>
      <form class="form" id="edit-goal">
        <div class="row"><label for="e-title">Название</label><input id="e-title" type="text" required maxlength="200" value="${esc(g.title)}"></div>
        <div class="row"><label for="e-target">Сумма, ₽</label><input id="e-target" type="number" min="1" step="1" required value="${g.target_rub}"></div>
        <div class="row"><label for="e-start">Старт</label><input id="e-start" type="datetime-local" required value="${toLocalInput(g.started_at)}"></div>
        <div class="row"><label for="e-end">Окончание</label><input id="e-end" type="datetime-local" value="${toLocalInput(g.ends_at)}"></div>
        <div class="row"><span></span><div class="actions"><button class="btn primary">Сохранить цель</button></div></div>
      </form>
    </div>

    <div class="card">
      <h2>Ручная корректировка</h2>
      <p class="hint">Для денег, которые пришли мимо платформ. Сейчас: <b id="gl-adjust"></b></p>
      <div class="inline">
        <input id="adj" type="number" step="1" placeholder="500" aria-label="Сумма корректировки, ₽">
        <button class="btn" id="adj-add">Добавить</button>
        <button class="btn" id="adj-sub">Вычесть</button>
      </div>
    </div>

    <div class="card">
      <h2>Завершить сбор</h2>
      <p class="hint">Цель уйдёт в историю с собранной суммой, виджет в OBS опустеет до новой цели.</p>
      <button class="btn danger" id="close-goal">Завершить сбор</button>
    </div>`;
  updateGoalNumbers(g, snap.collected_rub);

  $("#edit-goal").addEventListener("submit", (e) => {
    e.preventDefault();
    run(() => api("PATCH", "/api/goal", {
      title: $("#e-title").value.trim(),
      target_rub: Number($("#e-target").value),
      started_at: fromLocalInput($("#e-start").value),
      ends_at: fromLocalInput($("#e-end").value),
    }), "Цель сохранена");
  });
  const adjust = (sign) => run(async () => {
    const v = Number($("#adj").value);
    if (!v) throw new Error("Введите сумму");
    await api("PATCH", "/api/goal", { manual_adjust_rub: state.snapshot.goal.manual_adjust_rub + sign * v });
    $("#adj").value = "";
  }, sign > 0 ? "Сумма добавлена" : "Сумма вычтена");
  $("#adj-add").addEventListener("click", () => adjust(1));
  $("#adj-sub").addEventListener("click", () => adjust(-1));
  $("#close-goal").addEventListener("click", () => {
    if (confirm(`Завершить сбор «${g.title}»?`)) run(() => api("POST", "/api/goal/close"), "Сбор завершён");
  });
}

function updateGoalNumbers(g, collected) {
  $("#gl-title").textContent = g.title;
  $("#gl-sum").innerHTML = `${formatAmount(collected)} <small>/ ${formatAmount(g.target_rub)} ₽ · ${Math.floor((collected / g.target_rub) * 100)}%</small>`;
  $("#gl-meter").style.width = `${fillPercent(collected, g.target_rub)}%`;
  $("#gl-meta").innerHTML = `<span>Старт: ${fmtDate(g.started_at)}</span><span>Окончание: ${g.ends_at ? fmtDate(g.ends_at) : "не задано"}</span>`;
  $("#gl-adjust").textContent = `${g.manual_adjust_rub >= 0 ? "+" : ""}${formatAmount(g.manual_adjust_rub)} ₽`;
}

// ---------- design tab ----------

const POS3 = [["top", "Сверху"], ["bottom", "Снизу"], ["hidden", "Скрыто"]];
const FORMATS = ["percent", "amount", "amount_percent", "amount_target", "amount_target_percent"];
const WEIGHTS = [[300, "Светлый 300"], [400, "Обычный 400"], [500, "Средний 500"], [600, "Полужирный 600"], [700, "Жирный 700"], [800, "Очень жирный 800"], [900, "Чёрный 900"]];

const getPath = (obj, path) => path.split(".").reduce((o, k) => o[k], obj);
function setPath(obj, path, value) {
  const keys = path.split(".");
  const last = keys.pop();
  keys.reduce((o, k) => o[k], obj)[last] = value;
}

const fields = {
  elements: () => [
    { group: "Расположение" },
    { path: "elements.title_pos", label: "Заголовок сбора", type: "select", options: POS3 },
    { path: "elements.progress_pos", label: "Прогресс сбора", type: "select", options: [["inside", "Внутри полосы"], ...POS3] },
    { path: "elements.progress_format", label: "Вид прогресса", type: "select", options: FORMATS.map((f) => [f, formatProgress(5000, 10000, f)]) },
    { path: "elements.time_pos", label: "Оставшееся время", type: "select", options: POS3 },
    { path: "elements.text_align", label: "Выравнивание текста", type: "select", options: [["left", "По левому краю"], ["center", "По центру"]] },
    { group: "Дополнительно" },
    { path: "elements.show_bounds", label: "Границы сбора (0 и цель)", type: "toggle" },
    { path: "elements.backdrop", label: "Подложка", type: "toggle" },
    { path: "elements.backdrop_color", label: "Цвет подложки", type: "color", when: (s) => s.elements.backdrop },
  ],
  bar: () => [
    { path: "bar.height_vw", label: "Высота", type: "range", min: 0.5, max: 15, step: 0.1, unit: "vw" },
    { path: "bar.radius_vw", label: "Скругление", type: "range", min: 0, max: 8, step: 0.1, unit: "vw" },
    { path: "bar.border_vw", label: "Толщина обводки", type: "range", min: 0, max: 1, step: 0.05, unit: "vw" },
    { path: "bar.border_color", label: "Цвет обводки", type: "color", when: (s) => s.bar.border_vw > 0 },
    { path: "bar.bg", label: "Фон полосы", type: "paint" },
    { path: "bar.fill", label: "Заполнение", type: "paint" },
  ],
  fonts: () => [["title", "Заголовок сбора"], ["progress", "Прогресс сбора"], ["time", "Оставшееся время"]].flatMap(([k, name]) => [
    { group: name },
    { path: `fonts.${k}.family`, label: "Шрифт", type: "font" },
    { path: `fonts.${k}.size_vw`, label: "Размер", type: "range", min: 0.8, max: 10, step: 0.1, unit: "vw" },
    { path: `fonts.${k}.weight`, label: "Начертание", type: "select", options: WEIGHTS, number: true },
    { path: `fonts.${k}.italic`, label: "Курсив", type: "toggle" },
    { path: `fonts.${k}.color`, label: "Цвет", type: "color" },
    { path: `fonts.${k}.shadow`, label: "Тень", type: "toggle" },
    { path: `fonts.${k}.stroke_vw`, label: "Обводка текста", type: "range", min: 0, max: 0.5, step: 0.01, unit: "vw" },
    { path: `fonts.${k}.stroke_color`, label: "Цвет обводки", type: "color", when: (s) => getPath(s, `fonts.${k}.stroke_vw`) > 0 },
  ]),
};

let uid = 0;

function hexOf(color) {
  const m = /^#([0-9a-f]{6})$/i.exec(color);
  if (m) return color;
  const r = /^rgba?\((\d+),\s*(\d+),\s*(\d+)/i.exec(color);
  if (r) return "#" + [r[1], r[2], r[3]].map((n) => Number(n).toString(16).padStart(2, "0")).join("");
  return "#000000";
}

function withPickedHex(old, hex) {
  // Keep the alpha the user typed (rgba) when they pick a new hue from the native picker.
  const a = /^rgba\([^)]*,\s*([\d.]+)\)$/i.exec(old);
  if (!a) return hex;
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return `rgba(${r},${g},${b},${a[1]})`;
}

function colorControl(value, onChange) {
  const id = `c${uid++}`;
  const wrap = document.createElement("div");
  wrap.className = "color";
  wrap.innerHTML = `<input type="text" id="${id}" value="${esc(value)}" spellcheck="false"><label class="swatch"><span></span><input type="color" aria-label="Выбрать цвет"></label>`;
  const text = $("input[type=text]", wrap);
  const picker = $("input[type=color]", wrap);
  const chip = $(".swatch span", wrap);
  const sync = () => { chip.style.background = text.value; picker.value = hexOf(text.value); };
  sync();
  text.addEventListener("input", () => { sync(); onChange(text.value.trim()); });
  picker.addEventListener("input", () => { text.value = withPickedHex(text.value, picker.value); sync(); onChange(text.value); });
  return { el: wrap, id };
}

function buildControl(f) {
  const value = getPath(state.style, f.path);
  const set = (v) => { setPath(state.style, f.path, v); styleChanged(f); };
  const id = `f${uid++}`;
  let el;
  switch (f.type) {
    case "select": {
      el = document.createElement("select");
      el.id = id;
      el.innerHTML = f.options.map(([v, l]) => `<option value="${esc(v)}"${String(v) === String(value) ? " selected" : ""}>${esc(l)}</option>`).join("");
      el.addEventListener("change", () => set(f.number ? Number(el.value) : el.value));
      break;
    }
    case "toggle": {
      el = document.createElement("label");
      el.className = "switch";
      el.innerHTML = `<input type="checkbox" id="${id}"${value ? " checked" : ""}><span class="track"></span><span>${value ? "Включено" : "Отключено"}</span>`;
      const cb = $("input", el);
      cb.addEventListener("change", () => { $("span:last-child", el).textContent = cb.checked ? "Включено" : "Отключено"; set(cb.checked); });
      break;
    }
    case "range": {
      el = document.createElement("div");
      el.className = "range";
      el.innerHTML = `<input type="range" min="${f.min}" max="${f.max}" step="${f.step}" value="${value}" aria-label="${esc(f.label)}"><div class="unit" data-unit="${f.unit}"><input type="number" id="${id}" min="${f.min}" max="${f.max}" step="${f.step}" value="${value}"></div>`;
      const [slider, num] = el.querySelectorAll("input");
      slider.addEventListener("input", () => { num.value = slider.value; set(Number(slider.value)); });
      num.addEventListener("input", () => { if (num.value !== "" && Number.isFinite(Number(num.value))) { slider.value = num.value; set(Number(num.value)); } });
      break;
    }
    case "color": {
      const c = colorControl(value, set);
      el = c.el;
      return { el, id: c.id };
    }
    case "font": {
      el = document.createElement("input");
      el.type = "text";
      el.id = id;
      el.value = value;
      el.setAttribute("list", "font-list");
      el.placeholder = "Название шрифта";
      el.addEventListener("input", () => { if (el.value.trim()) set(el.value.trim()); });
      break;
    }
    case "paint": {
      el = document.createElement("div");
      el.className = "paint";
      const draw = () => {
        const p = getPath(state.style, f.path);
        el.innerHTML = "";
        const c1 = colorControl(p.color1, (v) => { p.color1 = v; styleChanged(); });
        el.append(c1.el);
        if (p.mode === "gradient") {
          el.append(colorControl(p.color2, (v) => { p.color2 = v; styleChanged(); }).el);
          const r = buildControl({ path: `${f.path}.angle`, label: "Угол", type: "range", min: 0, max: 360, step: 1, unit: "°" });
          el.append(r.el);
        }
        const toggle = document.createElement("button");
        toggle.type = "button";
        toggle.className = "link";
        toggle.textContent = p.mode === "gradient" ? "Использовать один цвет" : "Использовать градиент";
        toggle.addEventListener("click", () => { p.mode = p.mode === "gradient" ? "color" : "gradient"; draw(); styleChanged(); });
        el.append(toggle);
        el.firstChild.querySelector("input").id = id;
      };
      draw();
      break;
    }
  }
  return { el, id };
}

function renderStyleForm() {
  const form = $("#style-form");
  form.innerHTML = "";
  for (const f of fields[state.sub]()) {
    if (f.group) {
      const g = document.createElement("div");
      g.className = "group-title";
      g.textContent = f.group;
      form.append(g);
      continue;
    }
    if (f.when && !f.when(state.style)) continue;
    const row = document.createElement("div");
    row.className = "row";
    const { el, id } = buildControl(f);
    const label = document.createElement(f.type === "toggle" ? "span" : "label");
    label.className = "label";
    label.textContent = f.label;
    if (f.type !== "toggle") label.htmlFor = id;
    row.append(label, el);
    form.append(row);
  }
}

function styleChanged(field) {
  $("#dirty").hidden = JSON.stringify(state.style) === JSON.stringify(state.savedStyle);
  for (const b of document.querySelectorAll(".preset")) b.setAttribute("aria-pressed", "false");
  sendPreview();
  // Some fields show or hide others (e.g. backdrop color), so redraw when a toggle changes.
  if (field?.type === "toggle" || field?.type === "range" && /border_vw|stroke_vw/.test(field.path)) {
    const active = document.activeElement?.id;
    renderStyleForm();
    if (active && document.getElementById(active)) document.getElementById(active).focus();
  }
}

function previewSnapshot(style) {
  const snap = state.snapshot;
  if (snap?.goal) return { goal: snap.goal, collected_rub: snap.collected_rub, style };
  return { goal: SAMPLE_GOAL, collected_rub: SAMPLE_COLLECTED, style };
}

function sendPreview() {
  if (!state.style) return;
  $("#preview").contentWindow?.postMessage({ type: "preview", snapshot: previewSnapshot(state.style) }, location.origin);
}

function renderPresets() {
  const grid = $("#presets");
  grid.innerHTML = "";
  state.presets.forEach((p, i) => {
    const b = document.createElement("button");
    b.className = "preset";
    b.type = "button";
    b.setAttribute("aria-pressed", "false");
    b.innerHTML = `<span class="name">${esc(p.name)}</span><iframe src="/overlay?preview=1" tabindex="-1" aria-hidden="true" data-preset="${i}"></iframe>`;
    b.addEventListener("click", () => {
      state.style = clone(p.style);
      renderStyleForm();
      styleChanged();
      b.setAttribute("aria-pressed", "true");
    });
    grid.append(b);
  });
}

window.addEventListener("message", (e) => {
  if (e.origin !== location.origin || e.data?.type !== "preview-ready") return;
  if (e.source === $("#preview").contentWindow) return sendPreview();
  for (const f of document.querySelectorAll("iframe[data-preset]")) {
    if (f.contentWindow === e.source) {
      const style = state.presets[Number(f.dataset.preset)].style;
      f.contentWindow.postMessage({ type: "preview", snapshot: { goal: SAMPLE_GOAL, collected_rub: SAMPLE_COLLECTED, style } }, location.origin);
    }
  }
});

function initDesign() {
  for (const b of document.querySelectorAll(".subtabs button")) {
    b.addEventListener("click", () => {
      state.sub = b.dataset.sub;
      for (const o of document.querySelectorAll(".subtabs button")) o.setAttribute("aria-selected", String(o === b));
      renderStyleForm();
    });
  }
  $("#game-bg").addEventListener("change", (e) => $("#stage").classList.toggle("game", e.target.checked));
  $("#save-style").addEventListener("click", () => run(async () => {
    state.savedStyle = await api("PUT", "/api/style", state.style);
    state.style = clone(state.savedStyle);
    $("#dirty").hidden = true;
  }, "Дизайн сохранён"));
  $("#reset-style").addEventListener("click", () => {
    state.style = clone(state.savedStyle);
    renderStyleForm();
    styleChanged();
  });
  window.addEventListener("beforeunload", (e) => { if (!$("#dirty").hidden) e.preventDefault(); });
  $("#font-list").innerHTML = GOOGLE_FONTS.map((f) => `<option value="${esc(f)}">`).join("");
}

// ---------- history ----------

async function renderHistory() {
  const view = $("#history-view");
  try {
    const items = await api("GET", "/api/history");
    if (!items.length) {
      view.innerHTML = `<p class="empty">Завершённых сборов пока нет. Когда вы завершите текущую цель, она появится здесь.</p>`;
      return;
    }
    view.innerHTML = `
      <p class="hint">Всего сборов: ${items.length}</p>
      <table>
        <thead><tr><th>Цель сбора</th><th>Начало</th><th>Окончание</th></tr></thead>
        <tbody>${items.map((h) => `
          <tr>
            <td><div class="t">${esc(h.title)}</div>
                <div class="s">Собрано: <b class="${h.collected_rub > h.target_rub ? "over" : ""}">${formatAmount(h.collected_rub)}</b> / ${formatAmount(h.target_rub)} ₽</div></td>
            <td class="date">${fmtDate(h.started_at)}</td>
            <td class="date">${fmtDate(h.closed_at)}</td>
          </tr>`).join("")}
        </tbody>
      </table>`;
  } catch (e) {
    view.innerHTML = `<p class="err">${esc(e.message)}</p>`;
  }
}

// ---------- connections ----------

const STATE_TEXT = { off: "Не подключено", connecting: "Подключение…", ok: "Подключено", error: "Ошибка" };

function pill(p) {
  return `<span class="pill ${p.state}">${STATE_TEXT[p.state] ?? p.state}</span>`;
}

function renderSideStatus() {
  const p = state.platforms;
  if (!p) return;
  $("#side-status").innerHTML = `<div><span>DonationAlerts</span>${pill(p.da)}</div><div><span>DonateX</span>${pill(p.dx)}</div>`;
}

function renderConnections() {
  const p = state.platforms;
  const view = $("#connections-view");
  if (!p) return;
  if (view.dataset.ready) {
    // Refresh statuses only, keep anything typed in the fields.
    for (const name of ["da", "dx"]) {
      $(`#st-${name}`).innerHTML = pill(p[name]);
      $(`#er-${name}`).textContent = p[name].error ?? "";
    }
    $("#da-login").disabled = !p.da.has_client;
    return;
  }
  view.dataset.ready = "1";
  const overlayUrl = `${location.origin}/overlay`;
  view.innerHTML = `
    <div class="card">
      <h2>Ссылка для OBS</h2>
      <p class="hint">Источники → «+» → «Браузер», вставьте ссылку. Ширина 800, высота 200 — дальше масштабируйте как удобно, виджет растягивается вместе с источником.</p>
      <div class="copy"><code>${esc(overlayUrl)}</code><button class="btn" data-copy="${esc(overlayUrl)}">Копировать</button></div>
    </div>

    <div class="card">
      <div class="conn-head"><h2>DonateX</h2><span id="st-dx"></span></div>
      <p class="hint">Токен лежит в кабинете DonateX: Настройки → API и доступ.</p>
      <form class="inline" id="dx-form">
        <input type="password" id="dx-token" placeholder="${p.dx.configured ? "Токен сохранён — введите новый, чтобы заменить" : "Токен DonateX"}" autocomplete="off" style="flex:1;min-width:240px">
        <button class="btn primary">Сохранить токен</button>
      </form>
      <div class="err" id="er-dx"></div>
    </div>

    <div class="card">
      <div class="conn-head"><h2>DonationAlerts</h2><span id="st-da"></span></div>
      <ol class="steps">
        <li>Откройте <a href="https://www.donationalerts.com/application/clients" target="_blank" rel="noopener">donationalerts.com/application/clients</a> и нажмите «Создать новое приложение».</li>
        <li>Название любое. В поле Redirect URL вставьте <code>${esc(p.da.redirect_uri)}</code> <button type="button" class="link" data-copy="${esc(p.da.redirect_uri)}">копировать</button></li>
        <li>Скопируйте ID приложения и API ключ в поля ниже и сохраните.</li>
        <li>Нажмите «Войти через DonationAlerts» и разрешите доступ.</li>
      </ol>
      <form class="form" id="da-form">
        <div class="row"><label for="da-id">ID приложения</label><input type="text" id="da-id" value="${esc(p.da.client_id)}" autocomplete="off"></div>
        <div class="row"><label for="da-secret">API ключ</label><input type="password" id="da-secret" placeholder="${p.da.has_client ? "Ключ сохранён — введите новый, чтобы заменить" : ""}" autocomplete="off"></div>
        <div class="row"><span></span><div class="actions">
          <button class="btn">Сохранить</button>
          <button type="button" class="btn primary" id="da-login">Войти через DonationAlerts</button>
        </div></div>
      </form>
      <div class="err" id="er-da"></div>
    </div>

    <div class="card">
      <h2>Проверка виджета</h2>
      <p class="hint">Добавляет 100 ₽ к текущей цели, как будто пришёл донат. Чтобы убрать, вычтите сумму на вкладке «Текущая цель».</p>
      <button class="btn" id="test-donation">Тестовый донат 100 ₽</button>
    </div>`;

  view.addEventListener("click", (e) => {
    const c = e.target.closest("[data-copy]");
    if (c) navigator.clipboard.writeText(c.dataset.copy).then(() => toast("Скопировано"), () => toast("Не удалось скопировать", true));
  });
  $("#dx-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const token = $("#dx-token").value.trim();
    if (!token) return toast("Вставьте токен", true);
    run(async () => {
      state.platforms = await api("PUT", "/api/platforms/dx", { token });
      $("#dx-token").value = "";
      $("#dx-token").placeholder = "Токен сохранён — введите новый, чтобы заменить";
      renderConnections();
    }, "Токен DonateX сохранён");
  });
  $("#da-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const client_id = $("#da-id").value.trim();
    const client_secret = $("#da-secret").value.trim();
    if (!client_id || !client_secret) return toast("Заполните ID приложения и API ключ", true);
    run(async () => {
      state.platforms = await api("PUT", "/api/platforms/da", { client_id, client_secret });
      $("#da-secret").value = "";
      renderConnections();
    }, "Сохранено. Теперь войдите через DonationAlerts");
  });
  $("#da-login").addEventListener("click", () => { location.href = "/auth/da/start"; });
  $("#test-donation").addEventListener("click", () => {
    if (!state.snapshot?.goal) return toast("Сначала создайте цель", true);
    run(() => api("POST", "/api/debug/donation", { amount_rub: 100 }), "Тестовый донат добавлен");
  });
  renderConnections();
}

async function refreshPlatforms() {
  try {
    state.platforms = (await api("GET", "/api/state")).platforms;
    renderSideStatus();
    renderConnections();
  } catch {
    // server restarting; the next poll will catch up
  }
}

// ---------- live state ----------

function connectWs() {
  const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
  ws.onmessage = (e) => {
    state.snapshot = JSON.parse(e.data);
    renderGoal();
    sendPreview();
  };
  ws.onclose = () => setTimeout(connectWs, 2000);
}

async function init() {
  const [st, presets] = await Promise.all([api("GET", "/api/state"), api("GET", "/api/presets")]);
  state.platforms = st.platforms;
  state.savedStyle = st.style;
  state.style = clone(st.style);
  state.presets = presets;
  state.snapshot = { goal: st.goal, collected_rub: st.collected_rub, style: st.style };
  initDesign();
  renderPresets();
  renderStyleForm();
  renderGoal();
  renderSideStatus();
  window.addEventListener("hashchange", showTab);
  showTab();
  connectWs();
  setInterval(refreshPlatforms, 5000);
}

init().catch((e) => toast(`Не удалось загрузить настройки: ${e.message}`, true));
