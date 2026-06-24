//! Persisted user preferences (stored as JSON in the app config dir).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Verify the write by reading the device back and comparing checksums.
    pub validate: bool,
    /// Show a desktop notification when a flash finishes.
    pub notifications: bool,
    /// UI language code (e.g. "en", "de").
    #[serde(default = "default_language")]
    pub language: String,
    /// Most-recently-used image URLs (newest first), for quick re-selection.
    #[serde(default)]
    pub recent_urls: Vec<String>,
}

fn default_language() -> String {
    "en".to_string()
}

/// How many recent URLs we remember.
const RECENT_URLS_MAX: usize = 8;

impl Default for Settings {
    fn default() -> Self {
        Self {
            validate: true,
            notifications: true,
            language: default_language(),
            recent_urls: Vec::new(),
        }
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    settings_path(&app)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let path = settings_path(&app)?;
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// Record a successfully-used image URL at the front of the recent list
/// (de-duplicated, capped) and return the updated list.
#[tauri::command]
pub fn add_recent_url(app: AppHandle, url: String) -> Vec<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return get_settings(app).recent_urls;
    }
    let mut settings = get_settings(app.clone());
    settings.recent_urls.retain(|u| u != trimmed);
    settings.recent_urls.insert(0, trimmed.to_string());
    settings.recent_urls.truncate(RECENT_URLS_MAX);
    let _ = set_settings(app, settings.clone());
    settings.recent_urls
}
