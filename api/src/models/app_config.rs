use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use crate::db::DB;

pub mod keys {
    pub const SLSKD_API_KEY: &str = "slskd_api_key";
    pub const SLSKD_URL: &str = "slskd_url";
    pub const SLSKD_DOWNLOADS_DIR: &str = "slskd_download_dir";
    pub const DOWNLOAD_DIR: &str = "download_dir";
    pub const ALBUM_MODE: &str = "album_mode";
    pub const MUSICBRAINZ_SERVERS: &str = "musicbrainz_servers";
    pub const PLEX_CLIENT_ID: &str = "plex_client_id";
    pub const LIBRARY_SCAN_INTERVAL: &str = "library_scan_interval";
    pub const LIBRARY_WATCH_ENABLED: &str = "library_watch_enabled";
    pub const LIBRARY_REQUEST_RETRY_INTERVAL: &str = "library_request_retry_interval";
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "server", derive(sqlx::FromRow))]
pub struct AppConfig {
    pub key: String,
    pub value: String,
}

#[cfg(feature = "server")]
impl AppConfig {
    pub async fn get(key: &str) -> Result<Option<String>, String> {
        let row = sqlx::query_as::<_, Self>("SELECT * FROM app_config WHERE key = ?")
            .bind(key)
            .fetch_optional(&*DB)
            .await
            .map_err(|e| e.to_string())?;

        Ok(row.map(|r| r.value))
    }

    pub async fn set(key: &str, value: &str) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO app_config (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value"
        )
        .bind(key)
        .bind(value)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub async fn delete(key: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM app_config WHERE key = ?")
            .bind(key)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub async fn get_all() -> Result<Vec<Self>, String> {
        sqlx::query_as::<_, Self>("SELECT * FROM app_config ORDER BY key")
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn value_or(key: &str, default: &str) -> String {
        Self::get(key)
            .await
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default.to_string())
    }

    pub async fn download_path() -> std::path::PathBuf {
        std::path::PathBuf::from(Self::value_or(keys::DOWNLOAD_DIR, "/downloads").await)
    }

    pub async fn slskd_download_path() -> Option<std::path::PathBuf> {
        Self::get(keys::SLSKD_DOWNLOADS_DIR)
            .await
            .ok()
            .flatten()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
    }

    pub async fn is_album_mode() -> bool {
        Self::get(keys::ALBUM_MODE)
            .await
            .ok()
            .flatten()
            .map(|value| matches!(value.trim().to_lowercase().as_str(), "true" | "1" | "yes"))
            .unwrap_or(false)
    }

    pub async fn musicbrainz_servers() -> Vec<shared::metadata::MusicBrainzServer> {
        Self::get(keys::MUSICBRAINZ_SERVERS)
            .await
            .ok()
            .flatten()
            .and_then(|value| {
                serde_json::from_str::<Vec<shared::metadata::MusicBrainzServer>>(&value).ok()
            })
            .unwrap_or_else(shared::metadata::MusicBrainzServer::defaults)
    }
}
