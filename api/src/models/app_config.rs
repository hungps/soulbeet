use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use crate::db::DB;

pub mod keys {
    pub const SLSKD_API_KEY: &str = "slskd_api_key";
    pub const SLSKD_URL: &str = "slskd_url";
    pub const PLEX_CLIENT_ID: &str = "plex_client_id";
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
}
