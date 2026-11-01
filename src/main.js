/* ============================================================
   BETTERCLOUD — main.js
   Страницы, аудио, очередь, лайки, PIP, кнопки окна
   ============================================================ */

const T = window.__TAURI__;
const invoke = T.core.invoke;
const listen = T.event.listen;

/* ---------- shorthands ---------- */
const $ = (id) => document.getElementById(id);
const audio = $("audio");
const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) =>
    ({
      "\u0026": "\u0026amp;",
      "\u003C": "\u0026lt;",
      "\u003E": "\u0026gt;",
      '"': "\u0026quot;",
      "'": "\u0026#39;",
    }[c]),
  );

const fmtTime = (totalSec) => {
  const s = Math.max(0, Math.floor(totalSec || 0));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};

/** -large → -t500x500 (обложки SoundCloud масштабируются по суффиксу) */
const artUrl = (url, size = "t500x500") =>
  url ? url.replace(/-(large|t\d+x\d+|badge|small|tiny)\.(jpg|png)/, `-${size}.$2`) : null;

/** Состояние приложения */
const S = {
  status: null,
  page: "home",
  // очередь
  queue: [],
  pos: -1,
  history: [],
  shuffle: false,
  repeat: "off", // off | all | one
  seeking: false,
  // данные
  openPlaylist: null,
  likes: [],
  likesTotal: 0,
  likesLoadedPages: 0,
  likesDone: false,
  likesLoading: false,
  likedIds: new Set(),
  chartsCache: new Map(),
  waveLoading: false,
  // настройки
  theme: "dark",
  volume: 0.8,
  lastVolume: 0.8,
  genre: "all-music",
  // палитры
  paletteId: null,
  customPalettes: [],
  backgroundImage: null,
  backgroundOpacity: 0.35,
};

/** Готовые палитры: акцент/светлый акцент; фон и текст наследуются от темы. */
const PRESET_PALETTES = [
  { id: "sunset", name: "Закат", accent: "#ff5b23", accent2: "#ff8a3d" },
  { id: "ocean", name: "Океан", accent: "#2f9bff", accent2: "#6fc3ff" },
  { id: "mint", name: "Мята", accent: "#1fc98e", accent2: "#5fe8bb" },
  { id: "lavender", name: "Лаванда", accent: "#9a6cff", accent2: "#c2a3ff" },
  { id: "raspberry", name: "Малина", accent: "#ff4d7d", accent2: "#ff8aab" },
];

/** hex → rgba(r,g,b,a). */
function hexRgba(hex, alpha) {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

/** Применяет акценты и, для своей палитры, выбранный цвет фона. */
function applyBackground(image, opacity) {
  S.backgroundImage = image || null;
  S.backgroundOpacity = Math.min(1, Math.max(0, opacity ?? 0.35));
  const root = document.documentElement;
  root.style.setProperty(
    "--custom-bg-image",
    S.backgroundImage ? `url(${JSON.stringify(S.backgroundImage)})` : "none",
  );
  root.style.setProperty("--custom-bg-opacity", String(S.backgroundOpacity));

  const slider = $("background-opacity");
  const preview = $("background-preview");
  if (slider) {
    slider.value = Math.round(S.backgroundOpacity * 100);
    syncSlider(slider, slider.value);
  }
  if ($("background-opacity-value")) {
    $("background-opacity-value").textContent = `${Math.round(S.backgroundOpacity * 100)}%`;
  }
  if (preview) {
    preview.style.backgroundImage = S.backgroundImage
      ? `url(${JSON.stringify(S.backgroundImage)})`
      : "none";
    preview.classList.toggle("has-image", !!S.backgroundImage);
  }
  if ($("background-remove")) $("background-remove").disabled = !S.backgroundImage;
}

function applyPalette() {
  const root = document.documentElement;
  const preset = PRESET_PALETTES.find((p) => p.id === S.paletteId);
  const custom = S.customPalettes.find((p) => p.id === S.paletteId);
  const colors = preset
    ? { accent: preset.accent, accent2: preset.accent2 }
    : custom?.colors;
  root.style.removeProperty("--bg-0");
  root.style.removeProperty("--bg-1");
  if (!colors) {
    root.style.removeProperty("--accent");
    root.style.removeProperty("--accent-2");
    root.style.removeProperty("--accent-soft");
  } else {
    const a = colors.accent;
    const a2 = colors.accent2 || colors.accent;
    root.style.setProperty("--accent", a);
    root.style.setProperty("--accent-2", a2);
    root.style.setProperty("--accent-soft", hexRgba(a, 0.14));
    if (custom && colors.background) {
      root.style.setProperty("--bg-0", colors.background);
      root.style.setProperty("--bg-1", colors.background);
    }
  }
  document.querySelectorAll(".palette-tile").forEach((t) =>
    t.classList.toggle("active", t.dataset.paletteId === S.paletteId),
  );
}

const GENRES = [
  ["all-music", "Вся музыка"],
  ["electronic", "Электронная"],
  ["hip-hop", "Хип-хоп"],
  ["pop", "Поп"],
  ["rock", "Рок"],
  ["house", "Хаус"],
  ["techno", "Техно"],
  ["ambient", "Эмбиент"],
  ["jazz", "Джаз"],
  ["metal", "Метал"],
  ["indie", "Инди"],
  ["alternative", "Альтернатива"],
];

/* ---------- toast ---------- */
let toastEl = null;
let toastTimer = 0;
function toast(msg) {
  if (!toastEl) {
    toastEl = document.createElement("div");
    toastEl.className = "toast";
    document.body.appendChild(toastEl);
  }
  toastEl.textContent = msg;
  toastEl.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.remove("show"), 2600);
}

/* ============================================================
   СЕССИЯ / ПРОФИЛЬ / ТЕМА
   ============================================================ */

function applyTheme(theme) {
  S.theme = theme;
  document.documentElement.dataset.theme = theme;
  document.querySelectorAll(".theme-opt").forEach((b) =>
    b.classList.toggle("active", b.dataset.theme === theme),
  );
  applyPalette();
}

function applySession(st) {
  S.status = st;
  S.paletteId = st.paletteId ?? null;
  S.customPalettes = st.customPalettes ?? [];
  applyBackground(st.backgroundImage, st.backgroundOpacity);
  applyTheme(st.theme);
  renderPalettes();
  S.volume = st.volume;
  volumeInput.value = Math.round(st.volume * 100);
  syncSlider(volumeInput, st.volume * 100);

  const name = st.username || "Не выполнен вход";
  $("profile-name").textContent = name;
  $("profile-name").title = name;
  $("profile-sub").textContent = st.loggedIn ? "SoundCloud session" : "SoundCloud player";
  $("settings-profile").textContent = st.loggedIn ? name : "Не выполнен вход";

  const avatar = $("avatar-user");
  if (st.loggedIn) {
    avatar.textContent = name.trim().charAt(0).toUpperCase() || "♪";
    avatar.style.backgroundImage = st.avatarUrl
      ? `url(${artUrl(st.avatarUrl, "t200x200")})`
      : "";
    avatar.style.backgroundSize = "cover";
  } else {
    avatar.textContent = "?";
    avatar.style.backgroundImage = "";
  }

  $("status-dot").classList.toggle("on", st.loggedIn);
  $("status-label").textContent = st.loggedIn ? "подключён" : "не подключён";
  $("likes-count-label").textContent = st.loggedIn
    ? `В кеше: ${st.likesCached.toLocaleString("ru-RU")} треков`
    : "Войдите, чтобы увидеть коллекцию";

  // Если нет сессии — показываем логин, иначе — активную страницу.
  if (!st.loggedIn) {
    // сбрасываем кеши, зависящие от аккаунта
    playlistsCache = null;
    S.openPlaylist = null;
    switchPage("login");
  } else if (S.page === "login") {
    switchPage("home");
  }
}

async function refreshSession() {
  applySession(await invoke("session_status"));
}

/* ============================================================
   СТРАНИЦЫ
   ============================================================ */

const pageLoaders = {
  home: () => loadCharts(),
  wave: () => loadWave(),
  likes: () => loadLikes(),
  playlists: () => loadPlaylists(),
  search: () => $("global-search").focus(),
  settings: () => {},
  login: () => {},
};

function switchPage(name) {
  S.page = name;
  document.querySelectorAll(".page").forEach((p) =>
    p.classList.toggle("active", p.id === `page-${name}`),
  );
  document.querySelectorAll(".nav-item").forEach((n) =>
    n.classList.toggle("active", n.dataset.page === name),
  );
  // поиск в лайках виден только на странице избранного
  $("likes-search-wrap").style.visibility = name === "likes" ? "visible" : "hidden";
  pageLoaders[name]?.();
  $("content-scroll").scrollTop = 0;
}

function setStatus(id, msg, kind = "") {
  const el = $(id);
  if (!el) return;
  el.className = `page-status ${kind}`;
  el.textContent = msg || "";
}

/* ============================================================
   РЕНДЕР ТРЕКОВ
   ============================================================ */

const SVG_PLAY = '<svg class="icon-play" viewBox="0 0 24 24"><path d="m9 6 10 6-10 6z"/></svg>';
const SVG_PAUSE = '<svg class="icon-pause" viewBox="0 0 24 24"><path d="M8 6h3v12H8zm5 0h3v12h-3z"/></svg>';
const SVG_HEART =
  '<svg viewBox="0 0 24 24"><path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10Z"/></svg>';

function trackCard(t, index) {
  const art = artUrl(t.artworkUrl);
  const liked = S.likedIds.has(t.id);
  return `
  <div class="track-card" data-idx="${index}">
    <div class="track-cover">
      ${art ? `<img loading="lazy" src="${esc(art)}" alt=""/>` : ""}
      <div class="track-overlay">
        <button class="track-play" aria-label="Слушать">${S.playing && current()?.id === t.id ? SVG_PAUSE : SVG_PLAY}</button>
      </div>
    </div>
    <div class="track-meta">
      <div class="track-title" title="${esc(t.title)}">${esc(t.title)}</div>
      <div class="track-artist" title="${esc(t.username)}">${esc(t.username)}</div>
      <div class="track-foot">
        <span class="track-dur">${fmtTime(t.durationMs / 1000)}</span>
        <button class="like-btn ${liked ? "liked" : ""}" aria-label="В избранное">${SVG_HEART}</button>
      </div>
    </div>
  </div>`;
}

function likeRow(t, i) {
  const art = artUrl(t.artworkUrl, "t67x67");
  const liked = S.likedIds.has(t.id);
  return `
  <div class="like-row" data-idx="${i}">
    <div class="row-index">${i + 1}</div>
    ${art ? `<img class="row-cover" loading="lazy" src="${esc(art)}" alt=""/>` : '<div class="row-cover"></div>'}
    <div class="row-copy">
      <div class="row-title" title="${esc(t.title)}">${esc(t.title)}</div>
      <div class="row-artist" title="${esc(t.username)}">${esc(t.username)}</div>
    </div>
    <div class="row-dur">${fmtTime(t.durationMs / 1000)}</div>
    <button class="like-btn ${liked ? "liked" : ""}" aria-label="Убрать из избранного">${SVG_HEART}</button>
  </div>`;
}

function userCard(u) {
  const art = artUrl(u.avatarUrl, "t200x200");
  return `
  <div class="user-card" data-id="${u.id}">
    ${art ? `<img class="user-avatar" loading="lazy" src="${esc(art)}" alt=""/>` : '<div class="user-avatar"></div>'}
    <div class="user-copy">
      <div class="user-name" title="${esc(u.username)}">${esc(u.username)}</div>
      <div class="user-sub">${u.followersCount?.toLocaleString("ru-RU") ?? "—"} подписчиков</div>
    </div>
  </div>`;
}

function playlistCard(p) {
  const art = artUrl(p.artworkUrl, "t200x200");
  return `
  <div class="playlist-card" data-id="${p.id}">
    ${art ? `<img class="playlist-cover" loading="lazy" src="${esc(art)}" alt=""/>` : '<div class="playlist-cover"></div>'}
    <div class="playlist-copy">
      <div class="playlist-title" title="${esc(p.title)}">${esc(p.title)}</div>
      <div class="playlist-artist">${esc(p.username)} · ${p.trackCount ?? "?"} треков</div>
    </div>
  </div>`;
}

/** Навешивает клики на отрендеренный контейнер треков. */
function bindTrackList(container, tracks) {
  container.querySelectorAll(".track-card, .like-row").forEach((row) => {
    row.addEventListener("dblclick", () => playAt(tracks, +row.dataset.idx));
    row.querySelector(".track-play")?.addEventListener("click", (e) => {
      e.stopPropagation();
      playAt(tracks, +row.dataset.idx);
    });
    row.addEventListener("click", (e) => {
      if (e.target.closest(".like-btn")) return;
      playAt(tracks, +row.dataset.idx);
    });
    row.querySelector(".like-btn")?.addEventListener("click", async (e) => {
      const btn = e.currentTarget;
      const t = tracks[+row.dataset.idx];
      const like = !S.likedIds.has(t.id);
      try {
        await invoke("toggle_like", { track: t, like });
        like ? S.likedIds.add(t.id) : S.likedIds.delete(t.id);
        btn.classList.toggle("liked", like);
        btn.classList.remove("pop");
        void btn.offsetWidth;
        btn.classList.add("pop");
        syncLikeButton();
        toast(like ? "Добавлено в избранное ♥" : "Убрано из избранного");
        if (S.page === "likes" && !like) renderLikesFiltered();
        if (S.page === "likes") updateLikesCount();
      } catch (err) {
        toast(String(err).includes("not_logged_in") ? "Сначала войдите в аккаунт" : `Ошибка: ${err}`);
      }
    });
  });
}

/* ============================================================
   ЛАЙКИ (кеш + пагинация)
   ============================================================ */

async function loadLikes(force = false) {
  if (!S.status?.loggedIn) return setStatus("likes-status", "Войдите в аккаунт, чтобы видеть избранное", "error");
  if (S.likesLoading) return;
  S.likesLoading = true;
  setStatus("likes-status", "Загрузка коллекции…", "loading");
  try {
    if (force) {
      // перечитываем первую страницу (и фон докачает дальше)
      S.likesLoadedPages = 0;
      S.likes = [];
    }
    const [items, meta] = await invoke("likes_page", { page: S.likesLoadedPages });
    S.likes.push(...items);
    S.likesLoadedPages = meta.page + 1;
    S.likesTotal = meta.totalCached;
    S.likesDone = meta.done;
    // дедуп по id
    S.likes = S.likes.filter((t, i, arr) => arr.findIndex((x) => x.id === t.id) === i);
    items.forEach((t) => S.likedIds.add(t.id));
    renderLikesFiltered();
    updateLikesCount();
    setStatus("likes-status", S.likesDone ? "" : "Фоновая докачка…", "");
  } catch (e) {
    setStatus("likes-status", `Ошибка: ${e}`, "error");
  } finally {
    S.likesLoading = false;
  }
}

function updateLikesCount() {
  $("likes-count-label").textContent = `В кеше: ${Math.max(S.likesTotal, S.likes.length).toLocaleString("ru-RU")} треков`;
}

function renderLikesFiltered() {
  const q = $("likes-search").value.trim().toLowerCase();
  const words = q.split(/\s+/).filter(Boolean);
  const list = words.length
    ? S.likes.filter((t) => {
        const hay = `${t.title} ${t.username}`.toLowerCase();
        return words.every((w) => hay.includes(w));
      })
    : S.likes;
  const box = $("likes-tracks");
  box.innerHTML = list.map(likeRow).join("");
  bindTrackList(box, list);
  visibleLikes = list;
  markPlaying();
  setStatus("likes-status", list.length ? "" : q ? "Ничего не найдено" : "");
}

/* ============================================================
   ДОМ: ЧАРТЫ
   ============================================================ */

function renderGenreChips() {
  $("genre-chips").innerHTML = GENRES.map(
    ([slug, label]) =>
      `<button class="chip ${slug === S.genre ? "active" : ""}" data-genre="${slug}">${label}</button>`,
  ).join("");
  $("genre-chips").querySelectorAll(".chip").forEach((chip) =>
    chip.addEventListener("click", () => {
      S.genre = chip.dataset.genre;
      renderGenreChips();
      loadCharts();
    }),
  );
}

async function loadCharts() {
  const box = $("home-tracks");
  if (S.chartsCache.has(S.genre)) {
    box.innerHTML = S.chartsCache.get(S.genre).map(trackCard).join("");
    bindTrackList(box, S.chartsCache.get(S.genre));
    markPlaying();
    setStatus("home-status", "");
    return;
  }
  setStatus("home-status", "Загружаем популярное…", "loading");
  box.innerHTML = "";
  try {
    const tracks = await invoke("charts", { genre: S.genre });
    S.chartsCache.set(S.genre, tracks);
    box.innerHTML = tracks.map(trackCard).join("");
    bindTrackList(box, tracks);
    markPlaying();
    setStatus("home-status", tracks.length ? "" : "Пусто");
  } catch (e) {
    setStatus("home-status", `Ошибка: ${e}`, "error");
  }
}

/* ============================================================
   МОЯ ВОЛНА
   ============================================================ */

async function loadWave() {
  if (S.waveLoading) return;
  if (!S.status?.loggedIn) return setStatus("wave-status", "Волна строится на основе ваших лайков — войдите в аккаунт", "error");
  S.waveLoading = true;
  setStatus("wave-status", "Собираем рекомендации…", "loading");
  $("wave-tracks").innerHTML = "";
  try {
    const tracks = await invoke("my_wave");
    const box = $("wave-tracks");
    box.innerHTML = tracks.map(trackCard).join("");
    bindTrackList(box, tracks);
    waveTracks = tracks;
    markPlaying();
    setStatus("wave-status", tracks.length ? "" : "Полайкайте треки — и волна появится");
  } catch (e) {
    setStatus("wave-status", `Ошибка: ${e}`, "error");
  } finally {
    S.waveLoading = false;
  }
}

/* ============================================================
   ПЛЕЙЛИСТЫ
   ============================================================ */

async function loadPlaylists() {
  if (!S.status?.loggedIn) return setStatus("playlists-status", "Плейлисты доступны после входа", "error");
  S.openPlaylist = null;
  renderPlaylistsView();
}

let playlistsCache = null;

function renderPlaylistsView() {
  const cards = $("playlists-cards");
  const table = $("playlists-tracks");
  const back = $("pl-back");
  if (S.openPlaylist) {
    // Открытый плейлист: та же широкая таблица, что и в «Понравившихся».
    const p = S.openPlaylist;
    $("playlists-title").innerHTML = `${esc(p.title)}<span class="title-period">.</span>`;
    back.style.display = "";
    cards.style.display = "none";
    table.style.display = "";
    table.innerHTML = p.tracks.map((t, i) => likeRow(t, i)).join("");
    bindTrackList(table, p.tracks);
    setStatus("playlists-status", "");
    markPlaying();
    return;
  }
  $("playlists-title").innerHTML = `Плейлисты<span class="title-period">.</span>`;
  back.style.display = "none";
  table.style.display = "none";
  table.innerHTML = "";
  cards.style.display = "";
  if (!playlistsCache) {
    setStatus("playlists-status", "Загрузка…", "loading");
    invoke("my_playlists")
      .then((lists) => {
        playlistsCache = lists;
        renderPlaylistsView();
      })
      .catch((e) => setStatus("playlists-status", `Ошибка: ${e}`, "error"));
    return;
  }
  cards.innerHTML = playlistsCache.map(playlistCard).join("");
  cards.querySelectorAll(".playlist-card").forEach((card) =>
    card.addEventListener("click", () => openPlaylist(+card.dataset.id)),
  );
  setStatus("playlists-status", playlistsCache.length ? "" : "Плейлистов пока нет");
}

async function openPlaylist(id) {
  if (S.openPlaylist?.id === id) return;
  const meta = playlistsCache?.find((p) => p.id === id);
  S.openPlaylist = { id, title: meta?.title || "Плейлист", tracks: [] };
  renderPlaylistsView();
  setStatus("playlists-status", "Загружаем треки…", "loading");
  try {
    const tracks = await invoke("playlist_tracks", { playlistId: id });
    S.openPlaylist.tracks = tracks;
    setStatus("playlists-status", "");
    renderPlaylistsView();
  } catch (e) {
    S.openPlaylist = null;
    renderPlaylistsView();
    setStatus("playlists-status", `Ошибка: ${e}`, "error");
  }
}

/* ============================================================
   ГЛОБАЛЬНЫЙ ПОИСК
   ============================================================ */

let searchTimer = 0;
function scheduleGlobalSearch() {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(runGlobalSearch, 420);
}

async function runGlobalSearch() {
  const q = $("global-search").value.trim();
  if (!q) {
    $("search-tracks").innerHTML = "";
    $("search-users").innerHTML = "";
    $("search-playlists").innerHTML = "";
    setStatus("search-status", "");
    return;
  }
  setStatus("search-status", "Ищем по всему SoundCloud…", "loading");
  try {
    const res = await invoke("global_search", { query: q });
    const tb = $("search-tracks");
    tb.innerHTML = res.tracks.map(trackCard).join("");
    bindTrackList(tb, res.tracks);
    searchTracks = res.tracks;
    $("search-users").innerHTML = res.users.map(userCard).join("");
    $("search-playlists").innerHTML = res.playlists.map(playlistCard).join("");
    markPlaying();
    setStatus("search-status", res.tracks.length ? "" : "Треки не найдены");
  } catch (e) {
    setStatus("search-status", `Ошибка: ${e}`, "error");
  }
}

/* ============================================================
   ПЛЕЕР: ОЧЕРЕДЬ / АУДИО
   ============================================================ */

const current = () => S.queue[S.pos] ?? null;

function setQueue(tracks, startPos) {
  S.queue = [...tracks];
  S.pos = startPos;
  if (S.shuffle) shuffleQueueKeepingCurrent();
}

function shuffleQueueKeepingCurrent() {
  if (S.pos < 0) return;
  const cur = S.queue[S.pos];
  const rest = S.queue.filter((_, i) => i !== S.pos);
  for (let i = rest.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [rest[i], rest[j]] = [rest[j], rest[i]];
  }
  S.queue = [cur, ...rest];
  S.pos = 0;
}

const streamCache = new Map();
const STREAM_CACHE_TTL = 10 * 60 * 1000;
let playRequestId = 0;

function getStreamInfo(trackId) {
  const cached = streamCache.get(trackId);
  if (cached && Date.now() - cached.createdAt < STREAM_CACHE_TTL) return cached.promise;
  const promise = invoke("resolve_stream", { trackId }).catch((error) => {
    streamCache.delete(trackId);
    throw error;
  });
  streamCache.set(trackId, { createdAt: Date.now(), promise });
  return promise;
}

function prefetchNextStream() {
  if (!S.queue.length) return;
  const next = S.queue[(S.pos + 1) % S.queue.length];
  if (next && next.id !== current()?.id) getStreamInfo(next.id).catch(() => {});
}

async function playAt(tracks, index) {
  if (index < 0 || index >= tracks.length) return;
  const selected = tracks[index];
  if (current()?.id === selected.id) {
    togglePlay();
    return;
  }
  if (S.pos >= 0 && current()) S.history.push(S.pos);
  setQueue(tracks, index);
  await playCurrent();
}

async function playCurrent() {
  const t = current();
  if (!t) return;
  const requestId = ++playRequestId;
  audio.pause();
  $("player-title").textContent = t.title;
  $("player-title").title = t.title;
  $("player-artist").textContent = t.username;
  $("player-artist").title = t.username;

  const art = artUrl(t.artworkUrl);
  const img = $("player-art-img");
  img.removeAttribute("src");
  img.hidden = !art;
  if (art) img.src = art;

  try {
    const info = await getStreamInfo(t.id);
    if (requestId !== playRequestId || current()?.id !== t.id) return;
    if (window._hls) {
      window._hls.destroy();
      window._hls = null;
    }
    if (info.protocol === "hls" && window.Hls && window.Hls.isSupported()) {
      const hls = new window.Hls({
        startLevel: 0,
        maxBufferLength: 20,
        backBufferLength: 10,
      });
      window._hls = hls;
      hls.loadSource(info.url);
      hls.attachMedia(audio);
      hls.on(window.Hls.Events.MANIFEST_PARSED, () => {
        if (requestId === playRequestId) audio.play().catch(() => {});
      });
    } else {
      audio.src = info.url;
      audio.load();
      await audio.play();
    }
    prefetchNextStream();
  } catch (e) {
    if (requestId !== playRequestId) return;
    const msg = String(e);
    if (msg.includes("interrupted") || msg.includes("aborted") || e?.name === "AbortError") return;
    toast(`Не удалось открыть поток: ${e}`);
  }
}

function togglePlay() {
  // При hls.js media подключается без audio.src — ориентируемся на готовность.
  if (!audio.src && !(window._hls && window._hls.media)) {
    // если очередь была назначена (например, с волны) — запускаем первый
    if (S.queue.length) playCurrent();
    return;
  }
  if (audio.paused) audio.play().catch(() => {});
  else audio.pause();
}

function nextTrack(auto = false) {
  if (!S.queue.length) return;
  if (auto && S.repeat === "one") {
    audio.currentTime = 0;
    audio.play().catch(() => {});
    return;
  }
  if (S.pos < S.queue.length - 1) {
    S.pos++;
  } else if (S.repeat === "all" || !auto) {
    S.pos = 0;
  } else {
    // конец очереди
    audio.pause();
    return;
  }
  playCurrent();
}

function prevTrack() {
  if (!S.queue.length) return;
  if (audio.currentTime > 3) {
    audio.currentTime = 0;
    return;
  }
  S.pos = S.pos > 0 ? S.pos - 1 : S.queue.length - 1;
  playCurrent();
}

function markPlaying() {
  const cur = current();
  document.querySelectorAll(".track-card").forEach((c) => {
    const t = containerTrack(c);
    const isCurrent = !!t && cur?.id === t.id;
    c.classList.toggle("playing", isCurrent);
    const button = c.querySelector(".track-play");
    if (button) button.innerHTML = isCurrent && S.playing ? SVG_PAUSE : SVG_PLAY;
  });
  document.querySelectorAll(".like-row").forEach((r) => {
    const t = containerTrack(r);
    r.classList.toggle("playing", !!t && cur?.id === t.id);
  });
}

/** Трек, соответствующий строке, — по позиции в её контейнере. */
function containerTrack(row) {
  const container = row.parentElement;
  if (!container) return null;
  const list = currentListFor(container);
  return list?.[+row.dataset.idx] ?? null;
}

function currentListFor(container) {
  const id = container.id;
  const page = S.page;
  if (id === "home-tracks") return S.chartsCache.get(S.genre) ?? [];
  if (id === "wave-tracks") return waveTracks;
  if (id === "search-tracks") return searchTracks;
  if (id === "likes-tracks") return visibleLikes;
  if (id === "playlists-tracks") return S.openPlaylist?.tracks ?? [];
  return [];
}

/* ссылки на последние отрендеренные списки */
let waveTracks = [];
let searchTracks = [];
let visibleLikes = [];

/* ---------- аудио-события ---------- */

audio.addEventListener("play", () => {
  S.playing = true;
  playToggle.classList.add("playing");
  markPlaying();
  sendState();
});
audio.addEventListener("pause", () => {
  S.playing = false;
  playToggle.classList.remove("playing");
  markPlaying();
  sendState();
});
audio.addEventListener("ended", () => nextTrack(true));
audio.addEventListener("timeupdate", () => {
  if (S.seeking) return;
  const d = audio.duration || 0;
  const p = d ? (audio.currentTime / d) * 1000 : 0;
  progressInput.value = p;
  syncSlider(progressInput, p / 10);
  $("current-time").textContent = fmtTime(audio.currentTime);
  $("duration").textContent = fmtTime(d);
});
audio.addEventListener("loadedmetadata", () => {
  $("duration").textContent = fmtTime(audio.duration);
  sendState();
});
audio.addEventListener("volumechange", () => {
  muteBtn.classList.toggle("toggled", audio.muted || audio.volume === 0);
});

/* ---------- ползунки ---------- */

function syncSlider(input, percent) {
  input.style.setProperty("--fill", `${Math.min(100, Math.max(0, percent))}%`);
}

const progressInput = $("progress");
const volumeInput = $("volume");
const playToggle = $("play-toggle");
const muteBtn = $("mute-btn");

progressInput.addEventListener("input", () => {
  S.seeking = true;
  syncSlider(progressInput, progressInput.value / 10);
  $("current-time").textContent = fmtTime((progressInput.value / 1000) * (audio.duration || 0));
});
progressInput.addEventListener("change", () => {
  if (audio.duration) audio.currentTime = (progressInput.value / 1000) * audio.duration;
  S.seeking = false;
  sendState();
});
volumeInput.addEventListener("input", () => {
  const v = volumeInput.value / 100;
  S.volume = v;
  audio.muted = false;
  audio.volume = v;
  if (v > 0) S.lastVolume = v;
  syncSlider(volumeInput, volumeInput.value);
  schedulePersistVolume();
});
volumeInput.addEventListener("change", () => {
  invoke("set_volume", { volume: S.volume }).catch(() => {});
  sendState();
});

let volTimer = 0;
function schedulePersistVolume() {
  clearTimeout(volTimer);
  volTimer = setTimeout(() => invoke("set_volume", { volume: S.volume }).catch(() => {}), 800);
}

$("mute-btn").addEventListener("click", () => {
  audio.muted = !audio.muted;
  if (!audio.muted && audio.volume === 0) {
    audio.volume = S.lastVolume || 0.8;
    volumeInput.value = Math.round(audio.volume * 100);
    syncSlider(volumeInput, audio.volume * 100);
  }
});

/* ============================================================
   КНОПКИ ПЛЕЕРА
   ============================================================ */

playToggle.addEventListener("click", togglePlay);
$("next-track").addEventListener("click", () => nextTrack(false));
$("prev-track").addEventListener("click", prevTrack);
$("shuffle-btn").addEventListener("click", (e) => {
  S.shuffle = !S.shuffle;
  e.currentTarget.classList.toggle("toggled", S.shuffle);
  if (S.shuffle && S.queue.length) shuffleQueueKeepingCurrent();
  toast(S.shuffle ? "Перемешивание включено" : "Перемешивание выключено");
});
$("repeat-btn").addEventListener("click", (e) => {
  S.repeat = S.repeat === "off" ? "all" : S.repeat === "all" ? "one" : "off";
  const btn = e.currentTarget;
  btn.classList.toggle("toggled", S.repeat !== "off");
  btn.title = { off: "Повтор выключен", all: "Повтор очереди", one: "Повтор трека" }[S.repeat];
});

function syncLikeButton() {
  const cur = current();
  const liked = cur ? S.likedIds.has(cur.id) : false;
  const btn = $("like-current");
  btn.classList.toggle("liked", liked);
  btn.querySelector("svg").style.fill = liked ? "var(--accent)" : "none";
}

$("like-current").addEventListener("click", async () => {
  const cur = current();
  if (!cur) return;
  const like = !S.likedIds.has(cur.id);
  try {
    await invoke("toggle_like", { track: cur, like });
    like ? S.likedIds.add(cur.id) : S.likedIds.delete(cur.id);
    syncLikeButton();
    toast(like ? "Добавлено в избранное ♥" : "Убрано из избранного");
  } catch (e) {
    toast(`Ошибка: ${e}`);
  }
});

/* ============================================================
   КНОПКИ ОКНА / PIP
   ============================================================ */

$("btn-exit").addEventListener("click", () => invoke("app_exit"));
$("btn-fullscreen").addEventListener("click", async () => {
  const fs = await invoke("set_fullscreen");
  toast(fs ? "Полный экран включён" : "Полный экран выключен");
});
$("btn-minimize").addEventListener("click", () => invoke("minimize_main"));
$("pip-btn").addEventListener("click", async () => {
  await sendState(true);
  await invoke("open_mini_player");
  setTimeout(() => sendState(true), 120);
});

/* ============================================================
   НАВИГАЦИЯ / ЛОГИН / НАСТРОЙКИ
   ============================================================ */

document.querySelectorAll(".nav-item").forEach((n) =>
  n.addEventListener("click", () => switchPage(n.dataset.page)),
);

const openLogin = () => invoke("open_login_window").catch((e) => toast(`Ошибка: ${e}`));
$("login-btn").addEventListener("click", openLogin);
$("login-open").addEventListener("click", openLogin);
$("settings-login").addEventListener("click", openLogin);

$("settings-logout").addEventListener("click", async () => {
  await invoke("logout");
  S.likes = [];
  S.likedIds.clear();
  S.likesLoadedPages = 0;
  S.likesDone = false;
  S.queue = [];
  S.pos = -1;
  audio.removeAttribute("src");
  $("player-title").textContent = "—";
  $("player-artist").textContent = "—";
  toast("Вы вышли из аккаунта");
  refreshSession();
});

document.querySelectorAll(".theme-opt").forEach((btn) =>
  btn.addEventListener("click", async () => {
    applyTheme(btn.dataset.theme);
    const st = await invoke("set_theme", { theme: btn.dataset.theme });
    applySession(st);
  }),
);

/* ============================================================
   ПАЛИТРЫ ИНТЕРФЕЙСА
   ============================================================ */

/** Рендер сетки палитр (пресеты + свои). */
function renderPalettes() {
  const presetBox = $("preset-palettes");
  const customBox = $("custom-palettes");
  presetBox.innerHTML = PRESET_PALETTES.map(
    (p) => `
    <button class="palette-tile ${p.id === S.paletteId ? "active" : ""}" data-palette-id="${p.id}" title="Палитра «${esc(p.name)}»">
      <span class="palette-strip"><i style="background:${p.accent}"></i><i style="background:${p.accent2}"></i><i style="background:${hexRgba(p.accent, 0.18)}"></i></span>
      <b>${esc(p.name)}</b>
    </button>`,
  ).join("");
  const norm = (c) => (c && c.length === 7 ? c : "#888888");
  customBox.innerHTML = S.customPalettes.length
    ? S.customPalettes
        .map(
          (p) => `
    <button class="palette-tile ${p.id === S.paletteId ? "active" : ""}" data-palette-id="${esc(p.id)}" title="Палитра «${esc(p.name)}»">
      <span class="palette-strip"><i style="background:${esc(norm(p.colors.background))}"></i><i style="background:${esc(norm(p.colors.accent))}"></i><i style="background:${esc(norm(p.colors.accent2))}"></i></span>
      <b>${esc(p.name)}</b>
      <span class="palette-del" data-del="${esc(p.id)}" title="Удалить палитру">✕</span>
    </button>`,
        )
        .join("")
    : "";
}

/** Выбор палитры / удаление своей / сохранение новой / сброс. */
function bindBackgroundControls() {
  const fileInput = $("background-file");
  const opacityInput = $("background-opacity");

  fileInput.addEventListener("change", () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    if (!file.type.startsWith("image/")) {
      toast("Выберите изображение");
      fileInput.value = "";
      return;
    }
    if (file.size > 12 * 1024 * 1024) {
      toast("Файл слишком большой — максимум 12 МБ");
      fileInput.value = "";
      return;
    }
    const reader = new FileReader();
    reader.onload = async () => {
      try {
        const st = await invoke("set_background", {
          image: String(reader.result),
          opacity: S.backgroundOpacity,
        });
        applySession(st);
        toast("Задний фон сохранён");
      } catch (e) {
        toast(`Не удалось сохранить фон: ${e}`);
      } finally {
        fileInput.value = "";
      }
    };
    reader.onerror = () => {
      toast("Не удалось прочитать изображение");
      fileInput.value = "";
    };
    reader.readAsDataURL(file);
  });

  opacityInput.addEventListener("input", () => {
    const opacity = opacityInput.value / 100;
    applyBackground(S.backgroundImage, opacity);
  });
  opacityInput.addEventListener("change", async () => {
    try {
      const st = await invoke("set_background_opacity", {
        opacity: opacityInput.value / 100,
      });
      applySession(st);
    } catch (e) {
      toast(`Не удалось сохранить прозрачность: ${e}`);
    }
  });

  $("background-remove").addEventListener("click", async () => {
    try {
      const st = await invoke("set_background", {
        image: null,
        opacity: S.backgroundOpacity,
      });
      applySession(st);
      toast("Задний фон удалён");
    } catch (e) {
      toast(`Не удалось удалить фон: ${e}`);
    }
  });
}

function bindPaletteControls() {
  document.addEventListener("click", async (e) => {
    const del = e.target.closest(".palette-del");
    if (del) {
      e.stopPropagation();
      const st = await invoke("delete_custom_palette", { id: del.dataset.del });
      applySession(st);
      toast("Палитра удалена");
      return;
    }
    const tile = e.target.closest(".palette-tile");
    if (tile) {
      const st = await invoke("set_palette", { paletteId: tile.dataset.paletteId });
      applySession(st);
    }
  });

  $("palette-reset").addEventListener("click", async () => {
    const st = await invoke("set_palette", { paletteId: null });
    applySession(st);
    toast("Цвета темы по умолчанию");
  });

  $("palette-save").addEventListener("click", async () => {
    try {
      const st = await invoke("save_custom_palette", {
        name: $("palette-name").value,
        accent: $("palette-accent").value,
        accent2: $("palette-accent2").value,
        background: $("palette-background").value,
      });
      $("palette-name").value = "";
      applySession(st);
      toast("Палитра сохранена");
    } catch (e) {
      toast(String(e));
    }
  });
}

/* ---------- поиск в лайках ---------- */
$("likes-search").addEventListener("input", () => {
  $("likes-search-wrap").classList.toggle("has-text", !!$("likes-search").value);
  renderLikesFiltered();
});
$("likes-search-clear").addEventListener("click", () => {
  $("likes-search").value = "";
  $("likes-search-wrap").classList.remove("has-text");
  renderLikesFiltered();
  $("likes-search").focus();
});

/* ---------- глобальный поиск ---------- */
$("global-search").addEventListener("input", scheduleGlobalSearch);
$("global-search").addEventListener("keydown", (e) => {
  if (e.key === "Enter") runGlobalSearch();
});

/* ---------- обновить волну ---------- */
$("wave-refresh").addEventListener("click", () => {
  S.waveLoading = false;
  loadWave();
});

/* ============================================================
   СОСТОЯНИЕ → МИНИ-ПЛЕЕР, КОМАНДЫ ← МИНИ-ПЛЕЕР
   ============================================================ */

let lastStateSent = 0;
function sendState(force = false) {
  const now = Date.now();
  if (!force && now - lastStateSent < 250) return;
  lastStateSent = now;
  const cur = current();
  const css = getComputedStyle(document.documentElement);
  const color = (name) => css.getPropertyValue(name).trim();
  const st = {
    title: cur?.title ?? "—",
    artist: cur?.username ?? "—",
    playing: S.playing && !audio.paused,
    position: audio.currentTime || 0,
    duration: audio.duration || (cur?.durationMs ?? 0) / 1000,
    accent: color("--accent") || null,
    accent2: color("--accent-2") || null,
    background: color("--bg-0"),
    panel: color("--panel-solid"),
    line: color("--line"),
    text0: color("--text-0"),
    text1: color("--text-1"),
    text2: color("--text-2"),
    onAccent: color("--on-accent"),
    artwork: artUrl(cur?.artworkUrl),
    volume: audio.muted ? 0 : audio.volume,
    liked: cur ? S.likedIds.has(cur.id) : false,
  };
  return invoke("player_state", { state: st }).catch(() => {});
}
setInterval(() => sendState(), 1000);

listen("mini-cmd", ({ payload }) => {
  const { cmd, value } = payload;
  switch (cmd) {
    case "toggle":
      togglePlay();
      break;
    case "next":
      nextTrack(false);
      break;
    case "prev":
      prevTrack();
      break;
    case "close":
      invoke("close_mini_player");
      break;
    case "like":
      $("like-current").click();
      break;
    case "volume":
      audio.muted = false;
      audio.volume = value ?? 0.8;
      S.volume = audio.volume;
      if (audio.volume > 0) S.lastVolume = audio.volume;
      volumeInput.value = Math.round(audio.volume * 100);
      syncSlider(volumeInput, audio.volume * 100);
      schedulePersistVolume();
      break;
    case "seek":
      if (audio.duration) audio.currentTime = (value ?? 0) * audio.duration;
      break;
  }
});

/* ============================================================
   СОБЫТИЯ ОТ БЭКЕНДА
   ============================================================ */

listen("auth-done", ({ payload }) => {
  applySession(payload);
  toast(`Привет, ${payload.username ?? "друг"}!`);
  S.likes = [];
  S.likedIds.clear();
  S.likesLoadedPages = 0;
  S.likesDone = false;
  loadLikes();
});

listen("likes-updated", ({ payload }) => {
  S.likesTotal = payload ?? S.likesTotal;
  updateLikesCount();
  // Фон уже положил следующую страницу в кеш — сразу забираем её в UI.
  if (!S.likesLoading && S.likes.length < S.likesTotal) loadLikes();
});

listen("likes-done", () => {
  S.likesDone = true;
  setStatus("likes-status", "");
  if (!S.likesLoading && S.likes.length < S.likesTotal) loadLikes();
});

/* ============================================================
   СТАРТ
   ============================================================ */

async function init() {
  renderGenreChips();
  bindPaletteControls();
  bindBackgroundControls();
  $("pl-back").addEventListener("click", () => {
    S.openPlaylist = null;
    renderPlaylistsView();
  });
  try {
    const st = await invoke("session_status");
    applySession(st);
    if (st.loggedIn) {
      loadCharts();
      loadLikes();
    }
  } catch (e) {
    switchPage("login");
    setStatus("home-status", `Ошибка сессии: ${e}`, "error");
  }
}

init();
