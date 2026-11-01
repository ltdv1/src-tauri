//! Клиент SoundCloud (api-v2.soundcloud.com + классический api.soundcloud.com).
//! client_id добывается из JS-ассетов веб-плеера и ротируется при 401/403.

use crate::store::{Store, StoredTrack};
use regex::Regex;
use reqwest::{Client, Method, StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;
use std::time::Duration;

const API_V2: &str = "https://api-v2.soundcloud.com";
const API_V1: &str = "https://api.soundcloud.com";
const WEB_HOME: &str = "https://soundcloud.com/";

static SCRIPT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https://a-v2\.sndcdn\.com/assets/[^"'\s\\]+\.js"#).unwrap());
static CLIENT_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"client_id\s*[:=]\s*"([0-9a-zA-Z]{24,42})""#).unwrap());

pub static UA: LazyLock<String> = LazyLock::new(|| {
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36".into()
});

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScUser {
    pub id: u64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub followers_count: u64,
    pub permalink_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScPlaylist {
    pub id: u64,
    pub title: String,
    pub username: String,
    pub artwork_url: Option<String>,
    pub track_count: u64,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub tracks: Vec<StoredTrack>,
    pub users: Vec<ScUser>,
    pub playlists: Vec<ScPlaylist>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub url: String,
    pub protocol: String,
}

pub struct Sc {
    pub(crate) http: Client,
}

impl Sc {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent(UA.clone())
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client");
        Self { http }
    }

    // ---------- client_id ----------

    /// Достаёт client_id из ассетов веб-плеера. Сохранённый ранее — из настроек.
    pub async fn resolve_client_id(&self, store: &Store) -> Result<String, String> {
        // Guard не должен жить через await — забираем значение и отпускаем лок.
        let saved: Option<String> = {
            let s = store.settings.lock().unwrap();
            s.client_id.clone()
        };
        // Не проверяем сохранённый client_id отдельным сетевым запросом перед
        // каждым API-вызовом. Если он протух, api() увидит 401/403, обновит его
        // и повторит исходный запрос один раз.
        if let Some(id) = saved {
            return Ok(id);
        }
        let fresh = self.discover_client_id().await?;
        {
            let mut s = store.settings.lock().unwrap();
            s.client_id = Some(fresh.clone());
        }
        store.persist();
        Ok(fresh)
    }

    async fn discover_client_id(&self) -> Result<String, String> {
        let html = self
            .http
            .get(WEB_HOME)
            .header(header::ACCEPT, "text/html")
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;

        if let Some(m) = CLIENT_ID_RE.captures(&html) {
            if let Some(id) = m.get(1) {
                return Ok(id.as_str().to_string());
            }
        }

        let mut scripts: Vec<String> = SCRIPT_RE
            .find_iter(&html)
            .take(12)
            .map(|m| m.as_str().to_string())
            .collect();
        scripts.dedup();
        if scripts.is_empty() {
            return Err("Не нашёл JS-ассеты на soundcloud.com".into());
        }

        for script in scripts {
            let body = match self.http.get(&script).send().await {
                Ok(r) if r.status().is_success() => match r.text().await {
                    Ok(t) => t,
                    Err(_) => continue,
                },
                _ => continue,
            };
            if let Some(m) = CLIENT_ID_RE.captures(&body) {
                if let Some(id) = m.get(1) {
                    return Ok(id.as_str().to_string());
                }
            }
        }
        Err("client_id не найден в ассетах".into())
    }

    // ---------- базовый запрос ----------

    async fn api(
        &self,
        store: &Store,
        base: &str,
        path: &str,
        params: &[(String, String)],
        oauth: Option<&str>,
        method: Method,
    ) -> Result<Value, String> {
        let client_id = self.resolve_client_id(store).await?;
        let resp = self
            .request_once(store, base, path, params, oauth, &client_id, method.clone())
            .await
            .map_err(|e| e.to_string())?;

        // client_id умер — добываем новый и повторяем один раз.
        if resp.status() == StatusCode::UNAUTHORIZED
            || resp.status() == StatusCode::FORBIDDEN
            || resp.status().as_u16() == 550
        {
            let fresh = self.discover_client_id().await?;
            {
                let mut s = store.settings.lock().unwrap();
                if s.client_id.as_deref() == Some(client_id.as_str()) {
                    s.client_id = Some(fresh.clone());
                }
            }
            store.persist();
            let resp = self
                .request_once(store, base, path, params, oauth, &fresh, method)
                .await
                .map_err(|e| e.to_string())?;
            return Self::body_to_json(resp).await;
        }
        Self::body_to_json(resp).await
    }

    async fn request_once(
        &self,
        store: &Store,
        base: &str,
        path: &str,
        params: &[(String, String)],
        oauth: Option<&str>,
        client_id: &str,
        method: Method,
    ) -> Result<reqwest::Response, String> {
        // client_id нужен до запроса; для повторных попыток берём из настроек.
        let cid = if client_id.is_empty() {
            self.resolve_client_id(store).await?
        } else {
            client_id.to_string()
        };
        let mut url = reqwest::Url::parse(&format!("{base}{path}")).map_err(|e| e.to_string())?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("client_id", &cid);
            for (k, v) in params {
                qp.append_pair(k, v);
            }
            if let Some(token) = oauth {
                qp.append_pair("oauth_token", token);
            }
        }
        let mut req = self.http.request(method, url);
        req = req.header(header::ACCEPT, "application/json, text/plain, */*");
        req = req.header(header::ORIGIN, "https://soundcloud.com");
        if let Some(token) = oauth {
            req = req.header(header::AUTHORIZATION, format!("OAuth {token}"));
        }
        req.send().await.map_err(|e| e.to_string())
    }

    async fn body_to_json(resp: reqwest::Response) -> Result<Value, String> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("SoundCloud {status}: {}", truncate(&text, 300)));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    // ---------- профиль ----------

    pub async fn me(&self, store: &Store, token: &str) -> Result<ScUser, String> {
        let v = self
            .api(store, API_V1, "/me", &[], Some(token), Method::GET)
            .await?;
        Ok(ScUser {
            id: v["id"].as_u64().unwrap_or(0),
            username: v["username"].as_str().unwrap_or("").into(),
            avatar_url: avatar_big(v["avatar_url"].as_str()),
            followers_count: v["followers_count"].as_u64().unwrap_or(0),
            permalink_url: v["permalink_url"].as_str().map(str::to_string),
        })
    }

    /// Проверка токена через api-v2: JWT-токен из веб-сессии soundcloud.com
    /// (cookie oauth_token) валиден именно для v2; классический v1 /me
    /// такие токены отклоняет. client_id добавляем как у веб-плеера,
    /// но не валим запрос, если не удалось его получить.
    pub async fn me_v2(&self, store: &Store, token: &str) -> Result<ScUser, String> {
        let cid = self.resolve_client_id(store).await.unwrap_or_default();
        let mut url = reqwest::Url::parse(&format!("{API_V2}/me")).map_err(|e| e.to_string())?;
        if !cid.is_empty() {
            url.query_pairs_mut().append_pair("client_id", &cid);
        }
        let resp = self
            .http
            .get(url)
            .header(header::ACCEPT, "application/json, text/plain, */*")
            .header(header::ORIGIN, "https://soundcloud.com")
            .header(header::AUTHORIZATION, format!("OAuth {token}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("api-v2 /me -> {}", resp.status()));
        }
        let v = Self::body_to_json(resp).await?;
        Ok(ScUser {
            id: v["id"].as_u64().unwrap_or(0),
            username: v["username"].as_str().unwrap_or("").into(),
            avatar_url: avatar_big(v["avatar_url"].as_str()),
            followers_count: v["followers_count"].as_u64().unwrap_or(0),
            permalink_url: v["permalink_url"].as_str().map(str::to_string),
        })
    }

    // ---------- лайки ----------

    /// Одна страница лайков (50 штук, linked_partitioning). Возвращает (треки, next_href).
    /// Основа — публичный api-v2 эндпоинт /users/{uid}/likes: классический
    /// v1 /me/likes с веб-JWT даёт 403, v2 /me/likes — 404. Формат ответа
    /// (collection[].track + next_href) совпадает с прежним v1.
    pub async fn likes_page(
        &self,
        store: &Store,
        _token: &str,
        next: Option<String>,
    ) -> Result<(Vec<StoredTrack>, Option<String>), String> {
        let uid = store
            .settings
            .lock()
            .unwrap()
            .user_id
            .ok_or("no_user_id — лайки доступны после входа")?;
        let v = if let Some(href) = next {
            let mut url = reqwest::Url::parse(&href).map_err(|e| e.to_string())?;
            if !url.query_pairs().any(|(k, _)| k == "client_id") {
                let cid = self.resolve_client_id(store).await?;
                url.query_pairs_mut().append_pair("client_id", &cid);
            }
            let resp = self
                .http
                .get(url)
                .header(header::ACCEPT, "application/json")
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("likes page {}", resp.status()));
            }
            let text = resp.text().await.unwrap_or_default();
            serde_json::from_str(&text).unwrap_or(Value::Null)
        } else {
            self.api(
                store,
                API_V2,
                &format!("/users/{uid}/likes"),
                &[
                    ("limit".to_string(), "50".to_string()),
                    ("linked_partitioning".to_string(), "1".to_string()),
                ],
                None,
                Method::GET,
            )
            .await?
        };

        let mut tracks = Vec::new();
        if let Some(collection) = v["collection"].as_array() {
            for item in collection {
                // SoundCloud встречается в двух форматах: { track: {...} }
                // и непосредственно объект трека (/likes/tracks).
                if let Some(t) = parse_track(&item["track"]).or_else(|| parse_track(item)) {
                    tracks.push(t);
                }
            }
        }
        Ok((tracks, v["next_href"].as_str().map(str::to_string)))
    }

    /// Лайк / анлайк трека. Мутации принимает только классический v1
    /// (/me/likes/tracks/{id}); авторизация — строго через Cookie:
    /// вариант с Authorization/OAuth-параметром даёт 403.
    pub async fn set_track_like(
        &self,
        store: &Store,
        token: &str,
        track_id: u64,
        like: bool,
    ) -> Result<(), String> {
        let cid = self.resolve_client_id(store).await?;
        let url = format!("{API_V1}/me/likes/tracks/{track_id}?client_id={cid}");
        let method = if like { Method::PUT } else { Method::DELETE };
        let resp = self
            .http
            .request(method, &url)
            .header(header::ACCEPT, "application/json, text/plain, */*")
            .header(header::ORIGIN, "https://soundcloud.com")
            .header(header::COOKIE, format!("oauth_token={token}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let what = if like { "лайк" } else { "анлайк" };
            return Err(format!(
                "SoundCloud {}: {} не сохранён",
                resp.status(),
                what
            ));
        }
        Ok(())
    }

    // ---------- плейлисты пользователя ----------

    /// Плейлисты пользователя: api-v2 /users/{uid}/playlists (классический
    /// v1 /me/playlists с веб-JWT даёт 403). Ответ — обычный JSON-массив,
    /// поэтому дополнительно парсим collection на всякий случай.
    pub async fn my_playlists(
        &self,
        store: &Store,
        _token: &str,
    ) -> Result<Vec<ScPlaylist>, String> {
        let uid = store
            .settings
            .lock()
            .unwrap()
            .user_id
            .ok_or("no_user_id — плейлисты доступны после входа")?;
        let v = self
            .api(
                store,
                API_V2,
                &format!("/users/{uid}/playlists"),
                &[("limit".to_string(), "50".to_string())],
                None,
                Method::GET,
            )
            .await?;
        let mut out = Vec::new();
        let arr = if v.is_array() {
            v
        } else {
            v["collection"].clone()
        };
        if let Some(arr) = arr.as_array() {
            for p in arr {
                let track_count = p["track_count"]
                    .as_u64()
                    .unwrap_or_else(|| p["tracks"].as_array().map(|a| a.len() as u64).unwrap_or(0));
                out.push(ScPlaylist {
                    id: p["id"].as_u64().unwrap_or(0),
                    title: p["title"].as_str().unwrap_or("").into(),
                    username: p["user"]["username"].as_str().unwrap_or("").into(),
                    artwork_url: avatar_big(p["artwork_url"].as_str()),
                    track_count,
                });
            }
        }
        Ok(out)
    }

    /// Полный список треков плейлиста. SoundCloud присылает полные данные
    /// только для первых пяти элементов, а остальные оставляет ID-заглушками.
    /// Добираем эти объекты пакетами через /tracks?ids=... и сохраняем порядок.
    pub async fn playlist_tracks(
        &self,
        store: &Store,
        playlist_id: u64,
    ) -> Result<Vec<StoredTrack>, String> {
        let v = self
            .api(
                store,
                API_V2,
                &format!("/playlists/{playlist_id}"),
                &[("representation".to_string(), "full".to_string())],
                None,
                Method::GET,
            )
            .await?;
        let Some(items) = v["tracks"].as_array() else {
            return Ok(Vec::new());
        };

        let ids: Vec<u64> = items
            .iter()
            .filter_map(|item| item["id"].as_u64())
            .collect();
        let mut tracks: std::collections::HashMap<u64, StoredTrack> = items
            .iter()
            .filter_map(parse_track)
            .map(|track| (track.id, track))
            .collect();
        let missing: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|id| !tracks.contains_key(id))
            .collect();

        for chunk in missing.chunks(50) {
            let joined = chunk
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let batch = self
                .api(
                    store,
                    API_V2,
                    "/tracks",
                    &[("ids".to_string(), joined)],
                    None,
                    Method::GET,
                )
                .await?;
            if let Some(batch_items) = batch.as_array() {
                for track in batch_items.iter().filter_map(parse_track) {
                    tracks.insert(track.id, track);
                }
            }
        }

        Ok(ids
            .into_iter()
            .filter_map(|id| tracks.remove(&id))
            .collect())
    }

    // ---------- глобальный поиск ----------

    pub async fn search(
        &self,
        store: &Store,
        token: Option<&str>,
        query: &str,
        limit: u32,
    ) -> Result<SearchResults, String> {
        let params = [
            ("q".to_string(), query.to_string()),
            ("limit".to_string(), limit.to_string()),
            ("offset".to_string(), "0".to_string()),
        ];

        // Универсальный /search менял формат результатов и нередко возвращал
        // элементы без вложенных track/user/playlist. Типизированные
        // эндпоинты стабильнее и отдают сами объекты в collection.
        let tracks_v = self
            .api(store, API_V2, "/search/tracks", &params, token, Method::GET)
            .await?;
        let users_v = self
            .api(store, API_V2, "/search/users", &params, token, Method::GET)
            .await?;
        let playlists_v = match self
            .api(
                store,
                API_V2,
                "/search/playlists_without_albums",
                &params,
                token,
                Method::GET,
            )
            .await
        {
            Ok(v) => v,
            Err(_) => {
                self.api(
                    store,
                    API_V2,
                    "/search/playlists",
                    &params,
                    token,
                    Method::GET,
                )
                .await?
            }
        };

        let mut res = SearchResults::default();
        if let Some(collection) = tracks_v["collection"].as_array() {
            res.tracks.extend(collection.iter().filter_map(parse_track));
        }
        if let Some(collection) = users_v["collection"].as_array() {
            for u in collection {
                res.users.push(ScUser {
                    id: u["id"].as_u64().unwrap_or(0),
                    username: u["username"].as_str().unwrap_or("").into(),
                    avatar_url: avatar_big(u["avatar_url"].as_str()),
                    followers_count: u["followers_count"].as_u64().unwrap_or(0),
                    permalink_url: u["permalink_url"].as_str().map(str::to_string),
                });
            }
        }
        if let Some(collection) = playlists_v["collection"].as_array() {
            for p in collection {
                res.playlists.push(ScPlaylist {
                    id: p["id"].as_u64().unwrap_or(0),
                    title: p["title"].as_str().unwrap_or("").into(),
                    username: p["user"]["username"].as_str().unwrap_or("").into(),
                    artwork_url: avatar_big(p["artwork_url"].as_str()),
                    track_count: p["track_count"].as_u64().unwrap_or(0),
                });
            }
        }
        Ok(res)
    }

    /// Поиск только треков (для глобального поиска и добора в волну).
    pub async fn search_tracks(
        &self,
        store: &Store,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<StoredTrack>, String> {
        let v = self
            .api(
                store,
                API_V2,
                "/search/tracks",
                &[
                    ("q".to_string(), query.to_string()),
                    ("limit".to_string(), limit.to_string()),
                    ("offset".to_string(), offset.to_string()),
                ],
                None,
                Method::GET,
            )
            .await?;
        let mut out = Vec::new();
        if let Some(collection) = v["collection"].as_array() {
            for t in collection {
                if let Some(tr) = parse_track(t) {
                    out.push(tr);
                }
            }
        }
        Ok(out)
    }

    // ---------- чарты (Дом) ----------

    /// «Популярное» для Дома. Эндпоинт /charts жив только в виде
    /// kind=trending и только для all-music (kind=top отдаёт 404, прочие
    /// жанры — тоже). Поэтому: all-music → trending-чарт, остальные жанры
    /// (и фолбэк) — подбор поиском по названию жанра.
    pub async fn charts(
        &self,
        store: &Store,
        genre: &str,
        limit: u32,
    ) -> Result<Vec<StoredTrack>, String> {
        if genre == "all-music" {
            let genre_urn = format!("soundcloud:genres:{genre}");
            if let Ok(v) = self
                .api(
                    store,
                    API_V2,
                    "/charts",
                    &[
                        ("kind".to_string(), "trending".to_string()),
                        ("genre".to_string(), genre_urn),
                        ("limit".to_string(), limit.to_string()),
                    ],
                    None,
                    Method::GET,
                )
                .await
            {
                let mut out = Vec::new();
                if let Some(collection) = v["collection"].as_array() {
                    for item in collection {
                        if let Some(t) = parse_track(&item["track"]) {
                            out.push(t);
                        }
                    }
                }
                if !out.is_empty() {
                    return Ok(out);
                }
            }
        }
        // Фолбэк и все остальные жанры: поиск по ключевому слову жанра.
        let q = genre.replace('-', " ");
        self.search_tracks(store, &q, limit, 0).await
    }

    // ---------- related (для «Моей волны») ----------

    pub async fn related(
        &self,
        store: &Store,
        track_id: u64,
        limit: u32,
    ) -> Result<Vec<StoredTrack>, String> {
        let path = format!("/tracks/{track_id}/related");
        let v = self
            .api(
                store,
                API_V2,
                &path,
                &[("limit".to_string(), limit.to_string())],
                None,
                Method::GET,
            )
            .await?;
        let mut out = Vec::new();
        if let Some(collection) = v["collection"].as_array() {
            for t in collection {
                if let Some(tr) = parse_track(t) {
                    out.push(tr);
                }
            }
        }
        Ok(out)
    }

    // ---------- стрим ----------

    /// Прогрессивный mp3-URL для <audio>.
    pub async fn stream_url(
        &self,
        store: &Store,
        track_id: u64,
        token: Option<&str>,
    ) -> Result<StreamInfo, String> {
        let path = format!("/tracks/{track_id}");
        let v = self
            .api(store, API_V2, &path, &[], token, Method::GET)
            .await?;

        let auth = v["track_authorization"].as_str().unwrap_or("").to_string();
        let mut chosen: Option<(String, String)> = None; // (url, protocol)
        if let Some(list) = v["media"]["transcodings"].as_array() {
            for t in list {
                let protocol = t["format"]["protocol"].as_str().unwrap_or("");
                let url = t["url"].as_str().unwrap_or("");
                if url.is_empty() {
                    continue;
                }
                let mime = t["format"]["mime_type"].as_str().unwrap_or("");
                if protocol == "progressive" && mime == "audio/mpeg" {
                    chosen = Some((url.into(), "progressive".into()));
                    break;
                }
                if chosen.is_none() && protocol == "progressive" {
                    chosen = Some((url.into(), "progressive".into()));
                }
                if chosen.is_none() && protocol == "hls" {
                    chosen = Some((url.into(), "hls".into()));
                }
            }
        }
        let Some((transcoding_url, protocol)) = chosen else {
            return Err(
                "У трека нет доступных transcodings (приватный или заблокированный трек)".into(),
            );
        };

        let cid = self.resolve_client_id(store).await?;
        let mut url = reqwest::Url::parse(&transcoding_url).map_err(|e| e.to_string())?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("client_id", &cid);
            if !auth.is_empty() {
                qp.append_pair("track_authorization", &auth);
            }
            if let Some(t) = token {
                qp.append_pair("oauth_token", t);
            }
        }
        let resp = self
            .http
            .get(url)
            .header(header::ACCEPT, "application/json, text/plain, */*")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("transcoding {}", resp.status()));
        }
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(StreamInfo {
            url: v["url"].as_str().unwrap_or_default().into(),
            protocol,
        })
    }

    /// Загрузка бинарных данных (обложек).
    pub async fn http_get(&self, url: &str) -> Result<Vec<u8>, reqwest::Error> {
        self.http
            .get(url)
            .header(header::ACCEPT, "image/*, */*")
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
            .map(|b| b.to_vec())
    }
}

// ---------- утилиты ----------

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n { s } else { &s[..n] }
}

fn avatar_big(url: Option<&str>) -> Option<String> {
    url.map(|u| {
        u.replace("large.jpg", "t500x500.jpg")
            .replace("large.png", "t500x500.png")
    })
}

/// Приводит JSON трека api-v2/v1 к нашему StoredTrack.
pub fn parse_track(v: &Value) -> Option<StoredTrack> {
    let id = v["id"].as_u64()?;
    let title = v["title"].as_str()?.to_string();
    Some(StoredTrack {
        id,
        title,
        username: v["user"]["username"].as_str().unwrap_or("").into(),
        user_id: v["user"]["id"].as_u64().unwrap_or(0),
        artwork_url: avatar_big(
            v["artwork_url"]
                .as_str()
                .or_else(|| v["user"]["avatar_url"].as_str()),
        ),
        duration_ms: v["duration"].as_u64().unwrap_or(0),
        genre: v["genre"].as_str().map(str::to_string),
        permalink_url: v["permalink_url"].as_str().map(str::to_string),
    })
}

impl Default for Sc {
    fn default() -> Self {
        Self::new()
    }
}
