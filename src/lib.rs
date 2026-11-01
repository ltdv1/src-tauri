//! BETTERCLOUD — настольный SoundCloud-плеер на Tauri 2.
//! Модули: store (персистентность), soundcloud (API), lib (Tauri-команды).

mod soundcloud;
mod store;

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use soundcloud::{Sc, ScPlaylist, SearchResults, StreamInfo};
use std::sync::{Arc, Mutex};
use store::{LikesPage, Store, StoredTrack, Theme};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

#[derive(Clone)]
struct AppCtx {
    sc: Arc<Sc>,
    store: Arc<Store>,
    player_state: Arc<Mutex<Option<PlayerState>>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionStatus {
    logged_in: bool,
    username: Option<String>,
    avatar_url: Option<String>,
    user_id: Option<u64>,
    theme: String,
    volume: f64,
    likes_cached: usize,
    /// Активная палитра (None — дефисные цвета темы).
    palette_id: Option<String>,
    /// Пользовательские палитры (до 5).
    custom_palettes: Vec<store::Palette>,
    background_image: Option<String>,
    background_opacity: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageMeta {
    page: usize,
    total_cached: usize,
    done: bool,
    /// true если за этой страницей есть ещё (фон докачивает).
    has_more: bool,
}

fn status(ctx: &AppCtx) -> SessionStatus {
    let s = ctx.store.settings.lock().unwrap();
    SessionStatus {
        logged_in: s.oauth_token.is_some() && s.user_id.is_some(),
        username: s.username.clone(),
        avatar_url: s.avatar_url.clone(),
        user_id: s.user_id,
        theme: match s.theme {
            Theme::Dark => "dark".into(),
            Theme::Midnight => "midnight".into(),
            Theme::Light => "light".into(),
        },
        volume: s.volume,
        likes_cached: s.all_likes().len(),
        palette_id: s.palette_id.clone(),
        custom_palettes: s.custom_palettes.clone(),
        background_image: s.background_image.clone(),
        background_opacity: s.background_opacity,
    }
}

// ============================ базовые команды ============================

/// Красная кнопка: полный выход из приложения.
#[tauri::command]
fn app_exit(app: AppHandle) {
    app.exit(0);
}

/// Жёлтая кнопка: полный экран / обратно.
#[tauri::command]
fn set_fullscreen(app: AppHandle) -> Result<bool, String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    let next = !win.is_fullscreen().unwrap_or(false);
    win.set_fullscreen(next).map_err(|e| e.to_string())?;
    Ok(next)
}

/// Зелёная кнопка: на панель задач.
#[tauri::command]
fn minimize_main(app: AppHandle) -> Result<(), String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    win.minimize().map_err(|e| e.to_string())
}

// ============================ сессия / настройки ============================

#[tauri::command]
fn session_status(ctx: State<AppCtx>) -> SessionStatus {
    status(&ctx)
}

#[tauri::command]
fn set_theme(ctx: State<AppCtx>, theme: String) -> SessionStatus {
    {
        let mut s = ctx.store.settings.lock().unwrap();
        s.theme = match theme.as_str() {
            "midnight" => Theme::Midnight,
            "light" => Theme::Light,
            _ => Theme::Dark,
        };
    }
    ctx.store.persist();
    status(&ctx)
}

#[tauri::command]
fn set_volume(ctx: State<AppCtx>, volume: f64) {
    {
        let mut s = ctx.store.settings.lock().unwrap();
        s.volume = volume.clamp(0.0, 1.0);
    }
    ctx.store.persist();
}

#[tauri::command]
fn set_background(
    ctx: State<AppCtx>,
    image: Option<String>,
    opacity: f64,
) -> Result<SessionStatus, String> {
    let image = image.filter(|value| !value.is_empty());
    if let Some(value) = image.as_deref() {
        if !value.starts_with("data:image/") {
            return Err("Выберите изображение в формате JPG, PNG, WEBP или GIF".into());
        }
        if value.len() > 16 * 1024 * 1024 {
            return Err("Файл слишком большой — максимум 12 МБ".into());
        }
    }
    {
        let mut s = ctx.store.settings.lock().unwrap();
        s.background_image = image;
        s.background_opacity = opacity.clamp(0.0, 1.0);
    }
    ctx.store.persist();
    Ok(status(&ctx))
}

#[tauri::command]
fn set_background_opacity(ctx: State<AppCtx>, opacity: f64) -> SessionStatus {
    {
        let mut s = ctx.store.settings.lock().unwrap();
        s.background_opacity = opacity.clamp(0.0, 1.0);
    }
    ctx.store.persist();
    status(&ctx)
}

// ============================ палитры интерфейса ============================

/// Выбор активной палитры (пресет или custom-<id>). None — сброс к теме.
#[tauri::command]
fn set_palette(ctx: State<AppCtx>, palette_id: Option<String>) -> SessionStatus {
    {
        let mut s = ctx.store.settings.lock().unwrap();
        s.palette_id = palette_id.filter(|p| !p.is_empty());
    }
    ctx.store.persist();
    status(&ctx)
}

fn valid_hex(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Сохранение своей палитры (максимум 5) + немедленная активация.
#[tauri::command]
fn save_custom_palette(
    ctx: State<AppCtx>,
    name: String,
    accent: String,
    accent2: String,
    background: String,
) -> Result<SessionStatus, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Введите название палитры".into());
    }
    let (accent, accent2, background) = (
        accent.to_lowercase(),
        accent2.to_lowercase(),
        background.to_lowercase(),
    );
    if !valid_hex(&accent) || !valid_hex(&accent2) || !valid_hex(&background) {
        return Err("Цвет задаётся в формате #RRGGBB".into());
    }
    {
        let mut s = ctx.store.settings.lock().unwrap();
        if s.custom_palettes.len() >= 5 {
            return Err("Максимум 5 своих палитр — сначала удалите одну".into());
        }
        let mut colors = std::collections::BTreeMap::new();
        colors.insert("accent".to_string(), accent);
        colors.insert("accent2".to_string(), accent2);
        colors.insert("background".to_string(), background);
        let id = format!("custom-{}", now_ms());
        s.custom_palettes.push(store::Palette {
            id,
            name: name.to_string(),
            colors,
        });
        s.palette_id = Some(s.custom_palettes.last().unwrap().id.clone());
    }
    ctx.store.persist();
    Ok(status(&ctx))
}

/// Удаление своей палитры.
#[tauri::command]
fn delete_custom_palette(ctx: State<AppCtx>, id: String) -> SessionStatus {
    {
        let mut s = ctx.store.settings.lock().unwrap();
        s.custom_palettes.retain(|p| p.id != id);
        if s.palette_id.as_deref() == Some(id.as_str()) {
            s.palette_id = None;
        }
    }
    ctx.store.persist();
    status(&ctx)
}

#[tauri::command]
fn logout(app: AppHandle, ctx: State<AppCtx>) -> SessionStatus {
    {
        let mut s = ctx.store.settings.lock().unwrap();
        s.oauth_token = None;
        s.user_id = None;
        s.username = None;
        s.avatar_url = None;
        s.likes_pages.clear();
        s.likes_fetched_at_ms = None;
    }
    ctx.store.persist();
    if let Some(mini) = app.get_webview_window("mini") {
        let _ = mini.close();
    }
    status(&ctx)
}

// ============================ авторизация ============================

/// Дедуп + длина кандидата-токена.
fn push_candidate(list: &mut Vec<(String, String)>, name: &str, value: &str) {
    if value.len() >= 20 && !list.iter().any(|(n, _)| n == name) {
        list.push((name.to_string(), value.to_string()));
    }
}

/// Похож ли токен на JWT вида xxx.yyy.zzz (веб-токены SoundCloud).
fn looks_like_jwt(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() >= 5
                && p.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        })
}

/// Открывает встроенное окно обычного сайта soundcloud.com.
/// Токен добирается ДВУМЯ каналами (смотреть надо и тот и другой):
///   1. НАТИВНО: фоновый поток читает cookies окна из хранилища WebKit через
///      `cookies_for_url` — она видит HttpOnly/secure cookies, которые JS на
///      странице прочитать не может (классический cookie «OAuth Token»).
///   2. Резерв: JS-сканер в окне ищет JWT в localStorage/sessionStorage и
///      публикует его в заголовке окна маркером `BCTOKEN:` (если сайт держит
///      токен вне cookies).
/// Найденный кандидат проверяется запросом /me; при успехе окно закрывается.
#[tauri::command]
fn open_login_window(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("auth") {
        let _ = existing.set_focus();
        return Ok(());
    }
    // Резервный JS-сканер (второй канал; основной — cookies_for_url ниже).
    let scanner = r#"
(() => {
  if (window.__bcScan) return; window.__bcScan = true;
  const put = (t) => { const m = "BCTOKEN:" + t; if (document.title !== m) document.title = m; };
  const grab = () => {
    try {
      for (const store of [localStorage, sessionStorage]) {
        for (let i = 0; i < store.length; i++) {
          const k = store.key(i) || "";
          const v = String(store.getItem(k) || "");
          let mm = v.match(/((?:[A-Za-z0-9_-]{8,}\.){2}[A-Za-z0-9_-]{8,})/);
          if (!mm && /oauth|token/i.test(k)) mm = v.match(/((?:[A-Za-z0-9_-]{5,}\.){2}[A-Za-z0-9_-]{5,})/);
          if (mm) return mm[1];
        }
      }
    } catch (e) {}
    return null;
  };
  setInterval(() => { const t = grab(); if (t) put(t); }, 1000);
  const t0 = grab(); if (t0) put(t0);
})();
"#;
    WebviewWindowBuilder::new(
        &app,
        "auth",
        WebviewUrl::External(
            "https://soundcloud.com"
                .parse()
                .map_err(|e| format!("{e}"))?,
        ),
    )
    .title("Вход в SoundCloud")
    .inner_size(1100.0, 780.0)
    .min_inner_size(640.0, 500.0)
    .decorations(true)
    .initialization_script(scanner)
    .build()
    .map_err(|e| e.to_string())?;

    // Фоновый поток: поллим cookies окна (рекомендация Tauri — читать cookies
    // из отдельного потока, а не из sync-команды).
    let app2 = app.clone();
    std::thread::spawn(move || {
        let url: tauri::Url = "https://soundcloud.com".parse().expect("static url");
        let mut tried: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut logged_names = false;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(1200));
            let Some(win) = app2.get_webview_window("auth") else {
                return;
            };

            // Все cookies домена, включая HttpOnly и secure.
            let cookies = match win.cookies_for_url(url.clone()) {
                Ok(c) => c,
                Err(e) => {
                    println!("[auth] cookies_for_url error: {e}");
                    continue;
                }
            };
            // Кандидаты в порядке приоритета:
            //   1) cookie с точным именем oauth_token / "OAuth Token";
            //   2) BCTOKEN: из заголовка (резервный JS-сканер);
            //   3) прочие JWT-подобные cookies;
            //   4) oauth_refresh_token — в самом конце (он не для запросов к API).
            let mut candidates: Vec<(String, String)> = Vec::new();
            for c in &cookies {
                let (name, value) = (c.name(), c.value());
                if name.eq_ignore_ascii_case("oauth_token")
                    || name.eq_ignore_ascii_case("OAuth Token")
                {
                    push_candidate(&mut candidates, name, value);
                }
            }
            if let Ok(title) = win.title() {
                if let Some(t) = title.strip_prefix("BCTOKEN:") {
                    let t = t.trim();
                    if t.len() >= 20 {
                        push_candidate(&mut candidates, "title", t);
                    }
                }
            }
            for c in &cookies {
                let (name, value) = (c.name(), c.value());
                if name.to_ascii_lowercase().contains("refresh") {
                    continue;
                }
                if looks_like_jwt(value) {
                    push_candidate(&mut candidates, name, value);
                }
            }
            for c in &cookies {
                let (name, value) = (c.name(), c.value());
                if name.to_ascii_lowercase().contains("refresh") && looks_like_jwt(value) {
                    push_candidate(&mut candidates, name, value);
                }
            }
            if candidates.is_empty() {
                continue;
            }
            if !logged_names {
                let names: Vec<&str> = cookies.iter().map(|c| c.name()).collect();
                println!("[auth] cookies in store: {}", names.join(", "));
                logged_names = true;
            }

            let mut authorized = false;
            for (src, token) in candidates {
                if !tried.insert(token.clone()) {
                    continue;
                }
                println!("[auth] trying token from `{src}` (len {})", token.len());
                auth_with_token(app2.clone(), token);
                let ok = app2
                    .state::<AppCtx>()
                    .store
                    .settings
                    .lock()
                    .unwrap()
                    .user_id
                    .is_some();
                if ok {
                    authorized = true;
                    break;
                }
                println!("[auth] candidate `{src}` rejected — next");
            }
            if authorized {
                if let Some(w) = app2.get_webview_window("auth") {
                    let _ = w.close();
                }
                let _ = app2.get_webview_window("main").map(|m| {
                    let _ = m.show();
                    let _ = m.set_focus();
                });
                return;
            }
        }
    });
    Ok(())
}

/// Сохранение токена + профиль пользователя. user_id остаётся None, если /me не прошёл.
/// Веб-JWT из сессии soundcloud.com выдаётся под api-v2, поэтому проверяем
/// сначала v2, затем — классический v1 (ошибки пишем в лог для диагностики).
fn auth_with_token(app: AppHandle, token: String) {
    let ctx = app.state::<AppCtx>();
    let store = &ctx.store;
    let me_now = match tauri::async_runtime::block_on(ctx.sc.me_v2(store, &token)) {
        Ok(user) => Ok(user),
        Err(e2) => {
            println!("[auth] api-v2 /me failed: {e2} — trying api-v1");
            match tauri::async_runtime::block_on(sc_me_blocking(&ctx.sc, store, &token)) {
                Ok(user) => Ok(user),
                Err(e1) => {
                    println!("[auth] api-v1 /me failed: {e1}");
                    Err(e1)
                }
            }
        }
    };
    {
        let mut s = store.settings.lock().unwrap();
        match me_now {
            Ok(user) => {
                println!("[auth] authorized: {} (id {})", user.username, user.id);
                s.oauth_token = Some(token.clone());
                s.user_id = Some(user.id);
                s.username = Some(user.username);
                s.avatar_url = user.avatar_url;
                s.likes_pages.clear();
            }
            Err(_) => {
                s.oauth_token = Some(token.clone());
            }
        }
    }
    store.persist();
    let st = status(&ctx);
    let _ = app.emit("auth-done", &st);
}

/// Отдельная обёртка: async-вызов /me без удержания State в спавне.
async fn sc_me_blocking(sc: &Sc, store: &Store, token: &str) -> Result<ScUser, String> {
    sc.me(store, token).await
}

// Re-export для sc_me_blocking.
use soundcloud::ScUser;

// ============================ лайки ============================

/// Страница лайков из кеша; отсутствующая докачивается синхронно,
/// следующие — фоном (событие likes-updated).
#[tauri::command]
async fn likes_page(
    app: AppHandle,
    ctx: State<'_, AppCtx>,
    page: usize,
) -> Result<(Vec<StoredTrack>, PageMeta), String> {
    let token = {
        let s = ctx.store.settings.lock().unwrap();
        s.oauth_token.clone().ok_or("not_logged_in")?
    };

    let cached_exists = ctx.store.settings.lock().unwrap().likes_pages.len() > page;
    if !cached_exists {
        // Докачиваем последовательно до нужной страницы.
        fetch_likes_up_to(&ctx, &token, page).await?;
    }

    let s = ctx.store.settings.lock().unwrap();
    let items = s
        .likes_pages
        .get(page)
        .map(|p| p.items.clone())
        .unwrap_or_default();
    let done = s.likes_pages.get(page).map(|p| p.done).unwrap_or(true);
    let total = s.all_likes().len();
    drop(s);

    // Фоновая докачка следующих страниц.
    spawn_more_likes(app, ctx.inner().clone());
    Ok((
        items,
        PageMeta {
            page,
            total_cached: total,
            done,
            has_more: !done,
        },
    ))
}

/// Тянем страницы по цепочке, пока не появится запрошенная (или сеть не кончится).
async fn fetch_likes_up_to(ctx: &AppCtx, token: &str, page: usize) -> Result<(), String> {
    loop {
        let (have, next_of_last, last_done) = {
            let s = ctx.store.settings.lock().unwrap();
            let have = s.likes_pages.len();
            let last = s.likes_pages.last();
            (
                have,
                last.and_then(|p| p.next.clone()),
                last.map(|p| p.done).unwrap_or(false),
            )
        };
        if have > page || last_done {
            return Ok(());
        }
        let (items, next) = if let Some(href) = next_of_last {
            ctx.sc.likes_page(&ctx.store, token, Some(href)).await?
        } else {
            ctx.sc.likes_page(&ctx.store, token, None).await?
        };
        {
            let mut s = ctx.store.settings.lock().unwrap();
            s.likes_pages.push(LikesPage {
                items: items.clone(),
                done: next.is_none(),
                next: next.clone(),
            });
            s.likes_fetched_at_ms = Some(now_ms());
        }
        ctx.store.persist();
    }
}

/// Фоновая докачка: до 4 страниц за вызов.
fn spawn_more_likes(app: AppHandle, ctx: AppCtx) {
    tauri::async_runtime::spawn(async move {
        let token = match ctx.store.settings.lock().unwrap().oauth_token.clone() {
            Some(t) => t,
            None => return,
        };
        for _ in 0..4 {
            let (last_next, last_done) = {
                let s = ctx.store.settings.lock().unwrap();
                let last = s.likes_pages.last();
                (
                    last.and_then(|p| p.next.clone()),
                    last.map(|p| p.done).unwrap_or(true),
                )
            };
            if last_done || last_next.is_none() {
                let _ = app.emit("likes-done", ());
                return;
            }
            let (items, next) = match ctx.sc.likes_page(&ctx.store, &token, last_next).await {
                Ok(r) => r,
                Err(_) => return,
            };
            {
                let mut s = ctx.store.settings.lock().unwrap();
                s.likes_pages.push(LikesPage {
                    items: items.clone(),
                    done: next.is_none(),
                    next: next.clone(),
                });
                s.likes_fetched_at_ms = Some(now_ms());
            }
            ctx.store.persist();
            let total = ctx.store.settings.lock().unwrap().all_likes().len();
            let _ = app.emit("likes-updated", total);
        }
    });
}

/// Локальный мгновенный поиск по кешу лайков.
#[tauri::command]
fn search_likes(ctx: State<AppCtx>, query: String) -> Vec<StoredTrack> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let s = ctx.store.settings.lock().unwrap();
    s.all_likes()
        .into_iter()
        .filter(|t| {
            let hay = format!("{} {}", t.title, t.username).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .take(60)
        .collect()
}

/// Лайк/анлайк: сначала API, затем кеш. При анлайке трек исчезает из всех страниц кеша.
#[tauri::command]
async fn toggle_like(ctx: State<'_, AppCtx>, track: StoredTrack, like: bool) -> Result<(), String> {
    let token = {
        let s = ctx.store.settings.lock().unwrap();
        s.oauth_token.clone().ok_or("not_logged_in")?
    };
    ctx.sc
        .set_track_like(&ctx.store, &token, track.id, like)
        .await?;
    {
        let mut s = ctx.store.settings.lock().unwrap();
        if like {
            let present = s
                .likes_pages
                .iter()
                .any(|p| p.items.iter().any(|t| t.id == track.id));
            if !present {
                if let Some(first) = s.likes_pages.first_mut() {
                    first.items.insert(0, track);
                } else {
                    s.likes_pages.push(LikesPage {
                        items: vec![track],
                        done: false,
                        next: None,
                    });
                }
            }
        } else {
            for p in s.likes_pages.iter_mut() {
                p.items.retain(|t| t.id != track.id);
            }
        }
    }
    ctx.store.persist();
    Ok(())
}

// ============================ поиск / дом / волна ============================

#[tauri::command]
async fn global_search(ctx: State<'_, AppCtx>, query: String) -> Result<SearchResults, String> {
    // Поиск публичный: устаревший OAuth-токен превращает рабочий запрос в 401.
    ctx.sc.search(&ctx.store, None, &query, 20).await
}

#[tauri::command]
async fn search_tracks(
    ctx: State<'_, AppCtx>,
    query: String,
    offset: u32,
) -> Result<Vec<StoredTrack>, String> {
    ctx.sc.search_tracks(&ctx.store, &query, 40, offset).await
}

#[tauri::command]
async fn charts(ctx: State<'_, AppCtx>, genre: String) -> Result<Vec<StoredTrack>, String> {
    let genre = if genre.is_empty() {
        "all-music".to_string()
    } else {
        genre
    };
    ctx.sc.charts(&ctx.store, &genre, 40).await
}

/// «Моя волна»: related от случайных лайков + добор поиском по жанрам лайков.
#[tauri::command]
async fn my_wave(app: AppHandle, ctx: State<'_, AppCtx>) -> Result<Vec<StoredTrack>, String> {
    // Если кеш лайков пуст — синхронно докачиваем первую страницу,
    // иначе волна собиралась из нуля сидов (нужен был заход в «Понравившиеся»).
    let cache_empty = ctx.store.settings.lock().unwrap().all_likes().is_empty();
    if cache_empty {
        // Гард мьютекса должен умереть до await — клонируем токен в отдельном блоке.
        let token = {
            let s = ctx.store.settings.lock().unwrap();
            s.oauth_token.clone()
        };
        if let Some(token) = token {
            let _ = fetch_likes_up_to(&ctx, &token, 0).await;
            let total = {
                let s = ctx.store.settings.lock().unwrap();
                s.all_likes().len()
            };
            let _ = app.emit("likes-updated", total);
        }
    }
    let likes = ctx.store.settings.lock().unwrap().all_likes();
    if likes.is_empty() {
        return Ok(Vec::new());
    }

    // RNG не Send — вся работа с ним вне точек await.
    let seeds: Vec<StoredTrack> = pick_random_seeds(&likes, 4);
    let mut picked: Vec<StoredTrack> = Vec::new();

    for seed in seeds {
        let known: std::collections::HashSet<u64> = picked
            .iter()
            .map(|t| t.id)
            .chain(std::iter::once(seed.id))
            .collect();
        if let Ok(mut related) = ctx.sc.related(&ctx.store, seed.id, 20).await {
            related.retain(|t| !known.contains(&t.id));
            picked.append(&mut related);
        }
        if picked.len() >= 45 {
            break;
        }
    }

    if picked.len() < 25 {
        let mut genres: Vec<String> = likes.iter().filter_map(|t| t.genre.clone()).collect();
        genres.sort();
        genres.dedup();
        for g in genres.iter().take(3) {
            if let Ok(tracks) = ctx
                .sc
                .search_tracks(&ctx.store, &g.replace('/', " "), 15, 0)
                .await
            {
                let known: std::collections::HashSet<u64> = picked.iter().map(|t| t.id).collect();
                picked.extend(tracks.into_iter().filter(|t| !known.contains(&t.id)));
            }
            if picked.len() >= 35 {
                break;
            }
        }
    }

    shuffle_tracks(&mut picked);
    picked.truncate(50);
    Ok(picked)
}

/// Случайные N сидов из лайков (без ThreadRng в async-контексте).
fn pick_random_seeds(likes: &[StoredTrack], n: usize) -> Vec<StoredTrack> {
    let mut rng = rand::thread_rng();
    let mut copy = likes.to_vec();
    copy.shuffle(&mut rng);
    copy.truncate(n);
    copy
}

/// Перемешивание вне async (ThreadRng не Send).
fn shuffle_tracks(tracks: &mut [StoredTrack]) {
    let mut rng = rand::thread_rng();
    tracks.shuffle(&mut rng);
}

#[tauri::command]
async fn my_playlists(ctx: State<'_, AppCtx>) -> Result<Vec<ScPlaylist>, String> {
    let token = {
        let s = ctx.store.settings.lock().unwrap();
        s.oauth_token.clone().ok_or("not_logged_in")?
    };
    ctx.sc.my_playlists(&ctx.store, &token).await
}

/// Треки конкретного плейлиста (для открытия плейлиста из списка).
#[tauri::command]
async fn playlist_tracks(
    ctx: State<'_, AppCtx>,
    playlist_id: u64,
) -> Result<Vec<StoredTrack>, String> {
    ctx.sc.playlist_tracks(&ctx.store, playlist_id).await
}

// ============================ стрим ============================

#[tauri::command]
async fn resolve_stream(ctx: State<'_, AppCtx>, track_id: u64) -> Result<StreamInfo, String> {
    let token = ctx.store.settings.lock().unwrap().oauth_token.clone();
    ctx.sc
        .stream_url(&ctx.store, track_id, token.as_deref())
        .await
}

// ============================ обложка → цвет ============================

#[tauri::command]
async fn dominant_color(url: String) -> Result<String, String> {
    let fetcher = Sc::new();
    let bytes = fetcher.http_get(&url).await.map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let small = img.resize_exact(24, 24, image::imageops::FilterType::Triangle);

    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    let mut count = 0u64;
    for px in small.to_rgb8().pixels() {
        let (pr, pg, pb) = (px[0] as u64, px[1] as u64, px[2] as u64);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        if max - min < 12 {
            continue; // серые не считаем «цветом»
        }
        r += pr;
        g += pg;
        b += pb;
        count += 1;
    }
    if count == 0 {
        let (mut r2, mut g2, mut b2) = (0u64, 0u64, 0u64);
        for px in small.to_rgb8().pixels() {
            r2 += px[0] as u64;
            g2 += px[1] as u64;
            b2 += px[2] as u64;
        }
        let n = (24 * 24) as u64;
        return Ok(format!(
            "#{:02x}{:02x}{:02x}",
            (r2 / n) as u8,
            (g2 / n) as u8,
            (b2 / n) as u8
        ));
    }
    Ok(format!(
        "#{:02x}{:02x}{:02x}",
        (r / count) as u8,
        (g / count) as u8,
        (b / count) as u8
    ))
}

// ============================ мини-плеер (PIP) ============================

#[tauri::command]
fn open_mini_player(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("mini") {
        let _ = existing.set_size(tauri::Size::Logical(tauri::LogicalSize::new(360.0, 96.0)));
        let _ = existing.set_focus();
        return Ok(());
    }
    // Основное окно — на панель задач (как зелёная кнопка).
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.minimize();
    }

    let builder = WebviewWindowBuilder::new(&app, "mini", WebviewUrl::App("mini.html".into()))
        .title("BETTERCLOUD Mini")
        .inner_size(360.0, 96.0)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false);

    #[cfg(target_os = "macos")]
    let builder = builder.visible_on_all_workspaces(true);
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn close_mini_player(app: AppHandle) -> Result<(), String> {
    if let Some(mini) = app.get_webview_window("mini") {
        let _ = mini.close();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
    Ok(())
}

// ============================ стейт плеера: main → mini ============================

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub accent: Option<String>,
    pub accent2: Option<String>,
    pub background: String,
    pub panel: String,
    pub line: String,
    pub text0: String,
    pub text1: String,
    pub text2: String,
    pub on_accent: String,
    pub artwork: Option<String>,
    pub volume: f64,
    pub liked: bool,
}

/// Приём стейта из main, сохранение последней копии и пересылка в mini.
#[tauri::command]
fn player_state(app: AppHandle, ctx: State<AppCtx>, state: PlayerState) {
    if let Ok(mut current) = ctx.player_state.lock() {
        *current = Some(state.clone());
    }
    let _ = app.emit_to("mini", "player-state", &state);
}

/// Мини-плеер запрашивает состояние сам, чтобы не потерять первое событие.
#[tauri::command]
fn get_player_state(ctx: State<AppCtx>) -> Option<PlayerState> {
    ctx.player_state.lock().ok().and_then(|state| state.clone())
}

/// Команды управления из mini → main.
#[tauri::command]
fn mini_command(app: AppHandle, command: String, value: Option<f64>) {
    let _ = app.emit_to(
        "main",
        "mini-cmd",
        serde_json::json!({ "cmd": command, "value": value }),
    );
}

// ============================ misc ============================

#[tauri::command]
fn persist_store(ctx: State<AppCtx>) {
    ctx.store.persist();
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let store = Arc::new(Store::new(&handle));
            let sc = Arc::new(Sc::new());
            let player_state = Arc::new(Mutex::new(None));
            app.manage(AppCtx {
                sc,
                store,
                player_state,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_exit,
            set_fullscreen,
            minimize_main,
            session_status,
            set_theme,
            set_volume,
            set_background,
            set_background_opacity,
            set_palette,
            save_custom_palette,
            delete_custom_palette,
            logout,
            open_login_window,
            likes_page,
            toggle_like,
            search_likes,
            global_search,
            search_tracks,
            charts,
            my_wave,
            my_playlists,
            playlist_tracks,
            resolve_stream,
            dominant_color,
            open_mini_player,
            close_mini_player,
            player_state,
            get_player_state,
            mini_command,
            persist_store,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
