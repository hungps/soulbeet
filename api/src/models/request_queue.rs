use shared::download::DownloadQuery;
use uuid::Uuid;

use crate::db::DB;

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct LibraryRequestRow {
    pub id: String,
    pub user_id: String,
    pub folder_id: String,
    pub folder_path: String,
    pub query_json: String,
    pub label: String,
    pub artist: Option<String>,
    pub attempts: i64,
    pub last_attempt_at: Option<String>,
    pub created_at: String,
}

impl LibraryRequestRow {
    pub async fn enqueue(
        user_id: &str,
        folder_id: &str,
        folder_path: &str,
        query: &DownloadQuery,
        label: &str,
        artist: Option<&str>,
    ) -> Result<(), String> {
        let query_json = serde_json::to_string(query).map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO library_request_queue
                (id, user_id, folder_id, folder_path, query_json, label, artist, last_attempt_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(user_id, folder_id, label, artist) DO UPDATE SET
                query_json = excluded.query_json,
                folder_path = excluded.folder_path,
                last_attempt_at = CURRENT_TIMESTAMP",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(user_id)
        .bind(folder_id)
        .bind(folder_path)
        .bind(query_json)
        .bind(label)
        .bind(artist)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_all() -> Result<Vec<Self>, String> {
        sqlx::query_as::<_, Self>("SELECT * FROM library_request_queue ORDER BY created_at")
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn list_for_user(user_id: &str) -> Result<Vec<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT * FROM library_request_queue WHERE user_id = ? ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn mark_attempt(id: &str) -> Result<(), String> {
        sqlx::query(
            "UPDATE library_request_queue
             SET attempts = attempts + 1, last_attempt_at = CURRENT_TIMESTAMP
             WHERE id = ?",
        )
        .bind(id)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete(id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM library_request_queue WHERE id = ?")
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete_for_user(id: &str, user_id: &str) -> Result<bool, String> {
        let result = sqlx::query("DELETE FROM library_request_queue WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_matching(
        user_id: &str,
        label: &str,
        artist: Option<&str>,
    ) -> Result<(), String> {
        sqlx::query(
            "DELETE FROM library_request_queue
             WHERE user_id = ? AND label = ? AND (artist = ? OR (artist IS NULL AND ? IS NULL))",
        )
        .bind(user_id)
        .bind(label)
        .bind(artist)
        .bind(artist)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn query(&self) -> Result<DownloadQuery, String> {
        serde_json::from_str(&self.query_json).map_err(|e| e.to_string())
    }
}
