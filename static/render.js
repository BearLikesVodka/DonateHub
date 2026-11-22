// Shared widget renderer: used by the OBS overlay and by the settings preview (same file, same look).

export const GOOGLE_FONTS = [
  "Fira Sans Condensed", "Fira Sans", "Rubik", "Montserrat", "Oswald", "Unbounded", "Roboto",
  "Roboto Condensed", "Open Sans", "PT Sans", "PT Sans Narrow", "Russo One", "Exo 2", "Comfortaa",
  "Ubuntu", "Nunito", "Play", "Jura", "Press Start 2P", "Lobster", "Pacifico", "Manrope", "Inter",
  "Golos Text", "Onest", "Yanone Kaffeesatz", "Philosopher", "Kelly Slab", "Ruslan Display",
  "Neucha", "Marck Script", "Caveat", "Tektur", "Rubik Mono One", "Bad Script", "Seymour One",
];

export function formatAmount(n) {
  const v = Math.trunc(Number(n) || 0);
  const sign = v < 0 ? "-" : "";
  return sign + String(Math.abs(v)).replace(/\B(?=(\d{3})+(?!\d))/g, " ");
}

function percent(collected, target) {
  return target > 0 ? Math.floor((collected / target) * 100) : 0;
}

export function formatProgress(collected, target, format) {
  const p = `${percent(collected, target)}%`;
  const a = formatAmount(collected);
  const t = formatAmount(target);
  switch (format) {
    case "percent": return p;
    case "amount": return `${a} RUB`;
    case "amount_percent": return `${a} RUB (${p})`;
    case "amount_target": return `${a} / ${t} RUB`;
    default: return `${a} / ${t} RUB (${p})`;
  }
}

export function fillPercent(collected, target) {
  if (!(target > 0)) return 0;
  return Math.min(100, Math.max(0, (collected / target) * 100));
}

const pad = (n) => String(n).padStart(2, "0");

export function formatRemaining(endsAt, now) {
  if (!endsAt) return "";
  let s = Math.floor((endsAt.getTime() - now.getTime()) / 1000);
  if (s <= 0) return "Сбор завершён";
  const d = Math.floor(s / 86400);
  s %= 86400;
  const clock = `${pad(Math.floor(s / 3600))}:${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}`;
  return d > 0 ? `Осталось ${d} д ${clock}` : `Осталось ${clock}`;
}

function paint(p) {
  return p.mode === "gradient"
    ? `linear-gradient(${90 + Number(p.angle)}deg, ${p.color1}, ${p.color2})`
    : p.color1;
}

const loadedFonts = new Set();

function ensureFont(family) {
  if (typeof document === "undefined" || !GOOGLE_FONTS.includes(family) || loadedFonts.has(family)) return;
  loadedFonts.add(family);
  const link = document.createElement("link");
  link.rel = "stylesheet";
  // css v1 tolerates weights a family doesn't have; css2 rejects the whole request.
  link.href = `https://fonts.googleapis.com/css?family=${encodeURIComponent(family)}:300,400,400i,500,600,700,700i,800,800i,900&subset=cyrillic,latin&display=swap`;
  document.head.appendChild(link);
}

function fontVars(root, name, f) {
  ensureFont(f.family);
  const s = root.style;
  s.setProperty(`--${name}-family`, `"${f.family}", sans-serif`);
  s.setProperty(`--${name}-size`, `${f.size_vw}vw`);
  s.setProperty(`--${name}-weight`, f.weight);
  s.setProperty(`--${name}-style`, f.italic ? "italic" : "normal");
  s.setProperty(`--${name}-color`, f.color);
  s.setProperty(`--${name}-stroke`, `${f.stroke_vw}vw ${f.stroke_color}`);
  s.setProperty(`--${name}-shadow`, f.shadow ? "0 0.15vw 0.35vw rgba(0,0,0,0.65)" : "none");
}

function layoutKey(el) {
  return [el.title_pos, el.progress_pos, el.time_pos, el.show_bounds, el.backdrop].join("|");
}

function build(root, el) {
  const row = (pos) => {
    const parts = [];
    if (el.title_pos === pos) parts.push('<div class="title"></div>');
    if (el.progress_pos === pos) parts.push('<div class="progress"></div>');
    if (el.time_pos === pos) parts.push('<div class="time"></div>');
    return parts.join("");
  };
  root.innerHTML = `
    <div class="widget${el.backdrop ? " backdrop" : ""}">
      ${row("top")}
      <div class="bar">
        <div class="fill"></div>
        ${el.progress_pos === "inside" ? '<div class="progress inside"></div>' : ""}
      </div>
      ${el.show_bounds ? '<div class="bounds"><span class="lo">0</span><span class="hi"></span></div>' : ""}
      ${row("bottom")}
    </div>`;
  root.dataset.layout = layoutKey(el);
}

export function render(root, snapshot) {
  const { goal, collected_rub: collected, style } = snapshot;
  if (!goal) {
    root.innerHTML = "";
    delete root.dataset.layout;
    return;
  }
  const el = style.elements;
  if (root.dataset.layout !== layoutKey(el)) build(root, el);

  const s = root.style;
  const bar = style.bar;
  s.setProperty("--bar-h", `${bar.height_vw}vw`);
  s.setProperty("--bar-r", `${bar.radius_vw}vw`);
  s.setProperty("--bar-border", `${bar.border_vw}vw solid ${bar.border_color}`);
  s.setProperty("--bar-bg", paint(bar.bg));
  s.setProperty("--fill-bg", paint(bar.fill));
  s.setProperty("--backdrop", el.backdrop_color);
  s.setProperty("--align", el.text_align === "center" ? "center" : "flex-start");
  s.setProperty("--text-align", el.text_align === "center" ? "center" : "left");
  for (const name of ["title", "progress", "time"]) fontVars(root, name, style.fonts[name]);

  const q = (sel) => root.querySelector(sel);
  const fill = q(".fill");
  const width = `${fillPercent(collected, goal.target_rub)}%`;
  const prev = Number(root.dataset.collected);
  if (fill.style.width !== width) fill.style.width = width;
  if (Number.isFinite(prev) && collected > prev) {
    fill.classList.remove("pulse");
    void fill.offsetWidth; // restart the animation
    fill.classList.add("pulse");
  }
  root.dataset.collected = String(collected);
  root.dataset.endsAt = goal.ends_at || "";

  if (q(".title")) q(".title").textContent = goal.title;
  if (q(".progress")) q(".progress").textContent = formatProgress(collected, goal.target_rub, el.progress_format);
  if (q(".bounds .hi")) q(".bounds .hi").textContent = formatAmount(goal.target_rub);
  updateTime(root);
}

export function updateTime(root, now = new Date()) {
  const t = root.querySelector(".time");
  if (!t) return;
  const ends = root.dataset.endsAt ? new Date(root.dataset.endsAt) : null;
  t.textContent = formatRemaining(ends, now);
  t.hidden = !ends;
}

/** What the update banner on the settings page shows for `/api/update`, or null to hide it. */
export function updateBanner(info) {
  if (!info?.supported || !info.available) return null;
  if (info.installing) {
    return { text: `Устанавливается версия ${info.available}, программа перезапустится…`, button: null, busy: true };
  }
  return { text: `Доступна версия ${info.available}`, button: "Обновить", busy: false };
}
