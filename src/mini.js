/* ============================================================
   BETTERCLOUD — mini.js (мини-плеер PIP)
   Получает player-state от бэкенда, шлёт команды в main.
   ============================================================ */

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

const $ = (id) => document.getElementById(id);
const el = {
  art: $("m-art"),
  title: $("m-title"),
  artist: $("m-artist"),
  play: $("m-play"),
  prev: $("m-prev"),
  next: $("m-next"),
  like: $("m-like"),
  close: $("m-close"),
  progress: $("m-progress"),
  fill: $("m-progress-fill"),
  time: $("m-time"),
  volume: $("m-volume"),
};

const fmt = (s) => {
  const v = Math.max(0, Math.floor(s || 0));
  return `${Math.floor(v / 60)}:${String(v % 60).padStart(2, "0")}`;
};

/* ---------- приём стейта ---------- */

function applyState(payload) {
  if (!payload) return;
  el.title.textContent = payload.title;
  el.title.title = payload.title;
  el.artist.textContent = payload.artist;
  el.artist.title = payload.artist;
  el.play.classList.toggle("playing", payload.playing);
  el.like.classList.toggle("liked", payload.liked);

  if (payload.artwork) el.art.src = payload.artwork;
  else el.art.removeAttribute("src");

  const root = document.documentElement;
  const colors = {
    "--accent": payload.accent,
    "--accent-2": payload.accent2,
    "--bg": payload.background,
    "--panel": payload.panel,
    "--line": payload.line,
    "--text-0": payload.text0,
    "--text-1": payload.text1,
    "--text-2": payload.text2,
    "--on-accent": payload.onAccent,
  };
  for (const [name, value] of Object.entries(colors)) {
    if (value) root.style.setProperty(name, value);
  }

  const dur = payload.duration || 0;
  const pct = dur ? Math.min(100, (payload.position / dur) * 100) : 0;
  el.fill.style.width = `${pct}%`;
  el.time.textContent = fmt(payload.position);

  const v = Math.round((payload.volume ?? 0.8) * 100);
  el.volume.value = v;
  el.volume.style.setProperty("--fill", `${v}%`);
}

listen("player-state", ({ payload }) => applyState(payload));
invoke("get_player_state").then(applyState).catch(() => {});

/* ---------- команды в main ---------- */

const cmd = (command, value = null) => invoke("mini_command", { command, value });

el.play.addEventListener("click", () => cmd("toggle"));
el.next.addEventListener("click", () => cmd("next"));
el.prev.addEventListener("click", () => cmd("prev"));
el.like.addEventListener("click", () => cmd("like"));
el.close.addEventListener("click", () => cmd("close"));

el.volume.addEventListener("input", () => {
  const v = el.volume.value / 100;
  el.volume.style.setProperty("--fill", `${el.volume.value}%`);
  cmd("volume", v);
});

/* клик по полосе — перемотка (доля 0..1) */
el.progress.addEventListener("click", (e) => {
  const rect = el.progress.getBoundingClientRect();
  const frac = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
  cmd("seek", frac);
});

/* двойной клик по свободному месту — вернуть основное окно */
document.querySelector(".mini").addEventListener("dblclick", (e) => {
  if (e.target.closest("button, input")) return;
  cmd("close");
});
