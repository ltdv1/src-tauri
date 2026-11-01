//! Персистентное хранилище: настройки, OAuth-сессия, кеш лайков.
//! Всё лежит в appdata_dir/bettercloud-store.json в формате JSON.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// Тема интерфейса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Dark,
    Midnight,
    Light,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Dark
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredTrack {
    pub id: u64,
    pub title: String,
    pub username: String,
    pub user_id: u64,
    pub artwork_url: Option<String>,
    pub duration_ms: u64,
    pub genre: Option<String>,
    pub permalink_url: Option<String>,
}

/// Одна страница лайков (до 50 треков за запрос) + ссылка на следующую.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LikesPage {
    pub items: Vec<StoredTrack>,
    pub done: bool,
    /// next_href со страницы SoundCloud (для продолжения докачки).
    pub next: Option<String>,
}

/// Палитра интерфейса: имя + набор CSS-переменных (без `--`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub id: String,
    pub name: String,
    /// accent, accent2 и необязательный background.
    pub colors: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: Theme,
    pub volume: f64,
    pub oauth_token: Option<String>,
    pub user_id: Option<u64>,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    /// Значение client_id, которым последний раз успешно пользовались.
    pub client_id: Option<String>,
    /// Кеш лайков по страницам.
    pub likes_pages: Vec<LikesPage>,
    pub likes_fetched_at_ms: Option<u64>,
    /// Активная палитра: id пресета или custom-<ts>; None — только тема.
    pub palette_id: Option<String>,
    /// Пользовательские палитры (максимум 5).
    pub custom_palettes: Vec<Palette>,
    /// Пользовательский фон в формате data URL.
    pub background_image: Option<String>,
    /// Прозрачность пользовательского фона, 0..1.
    pub background_opacity: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            volume: 0.8,
            oauth_token: None,
            user_id: None,
            username: None,
            avatar_url: None,
            client_id: None,
            likes_pages: Vec::new(),
            likes_fetched_at_ms: None,
            palette_id: None,
            custom_palettes: Vec::new(),
            background_image: None,
            background_opacity: 0.35,
        }
    }
}

impl Settings {
    /// Все кешированные лайки одним списком (без дублей).
    pub fn all_likes(&self) -> Vec<StoredTrack> {
        let mut tracks = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for page in &self.likes_pages {
            for t in &page.items {
                if seen.insert(t.id) {
                    tracks.push(t.clone());
                }
            }
        }
        tracks
    }
}

/// Общий стейт приложения, доступный из Tauri-команд.
pub struct Store {
    pub(crate) path: PathBuf,
    pub settings: Mutex<Settings>,
}

impl Store {
    pub fn new(app: &tauri::AppHandle) -> Self {
        use tauri::Manager;
        let dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("bettercloud-store.json");
        let settings = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            settings: Mutex::new(settings),
        }
    }

    /// Сохранить текущее состояние на диск (вызывается после каждого изменения).
    pub fn persist(&self) {
        if let Ok(settings) = self.settings.lock() {
            let json = serde_json::to_string_pretty(&*settings).unwrap_or_default();
            let tmp = self.path.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(&tmp, &self.path);
            }
        }
    }
}
