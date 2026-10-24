use serde::{Deserialize, Serialize};
use shared::media_server::{MediaPathMapping, MediaServerKind};

#[cfg(feature = "server")]
use super::user_settings::UserSettings;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "server", derive(sqlx::FromRow))]
pub struct MediaServerConfig {
    pub user_id: String,
    pub kind: String,
    pub url: Option<String>,
    pub secure: bool,
    pub token: Option<String>,
    pub account_id: Option<String>,
    pub account_name: Option<String>,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub status: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "server", derive(sqlx::FromRow))]
pub struct MediaPathMappingRow {
    pub id: String,
    pub user_id: String,
    pub kind: String,
    pub local_path: String,
    pub remote_path: String,
}

impl MediaPathMappingRow {
    pub fn into_shared(self) -> MediaPathMapping {
        MediaPathMapping {
            id: self.id,
            kind: MediaServerKind::from(self.kind.as_str()),
            local_path: self.local_path,
            remote_path: self.remote_path,
        }
    }
}

#[cfg(feature = "server")]
impl MediaServerConfig {
    pub async fn get(user_id: &str, kind: MediaServerKind) -> Result<Option<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT * FROM media_server_configs WHERE user_id = ? AND kind = ?",
        )
        .bind(user_id)
        .bind(kind.as_str())
        .fetch_optional(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn list_for_user(user_id: &str) -> Result<Vec<Self>, String> {
        sqlx::query_as::<_, Self>("SELECT * FROM media_server_configs WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(&*crate::db::DB)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn link_account(
        user_id: &str,
        kind: MediaServerKind,
        token: &str,
        account_id: Option<&str>,
        account_name: Option<&str>,
        status: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO media_server_configs (user_id, kind, token, account_id, account_name, status, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(user_id, kind) DO UPDATE SET
                token = excluded.token,
                account_id = COALESCE(excluded.account_id, media_server_configs.account_id),
                account_name = COALESCE(excluded.account_name, media_server_configs.account_name),
                status = excluded.status,
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(user_id)
        .bind(kind.as_str())
        .bind(token)
        .bind(account_id)
        .bind(account_name)
        .bind(status)
        .execute(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn save_server(
        user_id: &str,
        kind: MediaServerKind,
        url: &str,
        secure: bool,
        server_id: Option<&str>,
        server_name: Option<&str>,
        status: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE media_server_configs
             SET url = ?, secure = ?, server_id = COALESCE(?, server_id),
                 server_name = COALESCE(?, server_name), status = ?, updated_at = CURRENT_TIMESTAMP
             WHERE user_id = ? AND kind = ?",
        )
        .bind(url)
        .bind(secure)
        .bind(server_id)
        .bind(server_name)
        .bind(status)
        .bind(user_id)
        .bind(kind.as_str())
        .execute(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn upsert_navidrome(
        user_id: &str,
        url: &str,
        secure: bool,
        token: &str,
        account_name: &str,
        status: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO media_server_configs (user_id, kind, url, secure, token, account_name, status, updated_at)
             VALUES (?, 'navidrome', ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(user_id, kind) DO UPDATE SET
                url = excluded.url,
                secure = excluded.secure,
                token = excluded.token,
                account_name = excluded.account_name,
                status = excluded.status,
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(user_id)
        .bind(url)
        .bind(secure)
        .bind(token)
        .bind(account_name)
        .bind(status)
        .execute(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn update_status(
        user_id: &str,
        kind: MediaServerKind,
        status: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE media_server_configs SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND kind = ?",
        )
        .bind(status)
        .bind(user_id)
        .bind(kind.as_str())
        .execute(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete(user_id: &str, kind: MediaServerKind) -> Result<(), String> {
        sqlx::query("DELETE FROM media_server_configs WHERE user_id = ? AND kind = ?")
            .bind(user_id)
            .bind(kind.as_str())
            .execute(&*crate::db::DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn any_configured(kind: MediaServerKind) -> Result<bool, String> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM media_server_configs WHERE kind = ?",
        )
        .bind(kind.as_str())
        .fetch_one(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())?;

        Ok(count > 0)
    }

    pub async fn find_user_by_account(
        kind: MediaServerKind,
        account_id: &str,
    ) -> Result<Option<String>, String> {
        sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM media_server_configs WHERE kind = ? AND account_id = ? LIMIT 1",
        )
        .bind(kind.as_str())
        .bind(account_id)
        .fetch_optional(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())
    }
}

#[cfg(feature = "server")]
impl MediaPathMappingRow {
    pub async fn list_for_user(
        user_id: &str,
        kind: Option<MediaServerKind>,
    ) -> Result<Vec<Self>, String> {
        match kind {
            Some(kind) => {
                sqlx::query_as::<_, Self>(
                    "SELECT * FROM media_path_mappings WHERE user_id = ? AND kind = ? ORDER BY local_path",
                )
                .bind(user_id)
                .bind(kind.as_str())
                .fetch_all(&*crate::db::DB)
                .await
                .map_err(|e| e.to_string())
            }
            None => {
                sqlx::query_as::<_, Self>(
                    "SELECT * FROM media_path_mappings WHERE user_id = ? ORDER BY kind, local_path",
                )
                .bind(user_id)
                .fetch_all(&*crate::db::DB)
                .await
                .map_err(|e| e.to_string())
            }
        }
    }

    pub async fn create(
        user_id: &str,
        kind: MediaServerKind,
        local_path: &str,
        remote_path: &str,
    ) -> Result<Self, String> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query_as::<_, Self>(
            "INSERT INTO media_path_mappings (id, user_id, kind, local_path, remote_path)
             VALUES (?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(&id)
        .bind(user_id)
        .bind(kind.as_str())
        .bind(local_path)
        .bind(remote_path)
        .fetch_one(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn update(
        id: &str,
        user_id: &str,
        local_path: &str,
        remote_path: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE media_path_mappings SET local_path = ?, remote_path = ? WHERE id = ? AND user_id = ?",
        )
        .bind(local_path)
        .bind(remote_path)
        .bind(id)
        .bind(user_id)
        .execute(&*crate::db::DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete(id: &str, user_id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM media_path_mappings WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(&*crate::db::DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(feature = "server")]
fn normalize_prefix(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_string()
}

#[cfg(feature = "server")]
fn replace_prefix(path: &str, from: &str, to: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let from = normalize_prefix(from);
    if from.is_empty() {
        return None;
    }
    if normalized == from {
        return Some(to.to_string());
    }
    let rest = normalized.strip_prefix(&from)?;
    if !rest.starts_with('/') {
        return None;
    }
    Some(format!("{}{}", normalize_prefix(to), rest))
}

#[cfg(feature = "server")]
pub async fn local_to_remote(
    user_id: &str,
    kind: MediaServerKind,
    local_path: &str,
) -> Result<Option<String>, String> {
    let mappings = MediaPathMappingRow::list_for_user(user_id, Some(kind)).await?;
    let mut best: Option<(usize, String)> = None;
    for mapping in mappings {
        if let Some(remote) = replace_prefix(local_path, &mapping.local_path, &mapping.remote_path)
        {
            let score = normalize_prefix(&mapping.local_path).len();
            if best.as_ref().is_none_or(|(current, _)| score > *current) {
                best = Some((score, remote));
            }
        }
    }
    Ok(best.map(|(_, path)| path))
}

#[cfg(feature = "server")]
pub async fn remote_to_local(
    user_id: &str,
    kind: MediaServerKind,
    remote_path: &str,
) -> Result<Option<String>, String> {
    let mappings = MediaPathMappingRow::list_for_user(user_id, Some(kind)).await?;
    let mut best: Option<(usize, String)> = None;
    for mapping in mappings {
        if let Some(local) = replace_prefix(remote_path, &mapping.remote_path, &mapping.local_path)
        {
            let score = normalize_prefix(&mapping.remote_path).len();
            if best.as_ref().is_none_or(|(current, _)| score > *current) {
                best = Some((score, local));
            }
        }
    }
    Ok(best.map(|(_, path)| path))
}

#[cfg(feature = "server")]
pub async fn reset_playlists_for_server_change(user_id: &str) {
    let _ = UserSettings::clear_all_discovery_playlist_ids(user_id).await;
}
