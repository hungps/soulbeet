#[cfg(feature = "server")]
use crate::db::DB;
#[cfg(feature = "server")]
use shared::download::DownloadProgress;
use shared::download::DownloadState;
#[cfg(feature = "server")]
use shared::download::{DownloadRequestContext, DownloadableItem};
#[cfg(feature = "server")]
use shared::download::{DownloadRequestSummary, DownloadRequestTrack};
#[cfg(feature = "server")]
use shared::library::{album_activity_key, normalize_key, track_activity_key, DownloadActivity};

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "server", derive(sqlx::FromRow))]
pub struct DownloadRequestRow {
    pub id: String,
    pub user_id: String,
    pub request_key: String,
    pub label: String,
    pub artist: Option<String>,
    pub target_folder: String,
    pub backend: Option<String>,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "server", derive(sqlx::FromRow))]
pub struct DownloadRequestTrackRow {
    pub id: i64,
    pub request_id: String,
    pub item_key: String,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub size: i64,
    pub quality: String,
    pub quality_score: f64,
    pub duration: Option<i64>,
    pub backend_data: Option<String>,
    pub transfer_id: Option<String>,
    pub state: String,
    pub percent: f64,
    pub transferred: i64,
    pub speed: f64,
    pub error: Option<String>,
    pub local_path: Option<String>,
    pub import_output: Option<String>,
    pub retryable: i64,
    pub can_choose_import: i64,
    pub queue_position: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn state_key(state: &DownloadState) -> &'static str {
    match state {
        DownloadState::Queued => "queued",
        DownloadState::InProgress => "in_progress",
        DownloadState::Completed => "completed",
        DownloadState::ImportQueued(_) => "import_queued",
        DownloadState::Importing => "importing",
        DownloadState::Imported => "imported",
        DownloadState::ImportSkipped => "import_skipped",
        DownloadState::Failed(_) => "failed",
        DownloadState::Cancelled => "cancelled",
        DownloadState::Stopped => "stopped",
    }
}

pub fn state_from_key(state: &str, error: Option<&str>) -> DownloadState {
    match state {
        "in_progress" => DownloadState::InProgress,
        "completed" => DownloadState::Completed,
        "import_queued" => DownloadState::ImportQueued(1),
        "importing" => DownloadState::Importing,
        "imported" => DownloadState::Imported,
        "import_skipped" => DownloadState::ImportSkipped,
        "failed" => DownloadState::Failed(error.unwrap_or_default().to_string()),
        "cancelled" => DownloadState::Cancelled,
        "stopped" => DownloadState::Stopped,
        _ => DownloadState::Queued,
    }
}

fn queue_position(state: &DownloadState) -> Option<i64> {
    match state {
        DownloadState::ImportQueued(ahead) => Some(i64::from(*ahead)),
        _ => None,
    }
}

pub fn state_is_terminal_key(state: &str) -> bool {
    matches!(
        state,
        "imported" | "import_skipped" | "failed" | "cancelled" | "stopped"
    )
}

#[cfg(feature = "server")]
async fn track_in_library(user_id: &str, artist: &str, title: &str) -> Result<bool, String> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM library_tracks t
            JOIN folders f ON f.id = t.folder_id
            WHERE f.user_id = ? AND t.artist_key = ? AND t.title_key = ?
        )",
    )
    .bind(user_id)
    .bind(normalize_key(artist))
    .bind(normalize_key(title))
    .fetch_one(&*DB)
    .await
    .map_err(|e| e.to_string())?;
    Ok(exists != 0)
}

impl DownloadRequestTrackRow {
    #[cfg(feature = "server")]
    pub fn to_shared(&self) -> DownloadRequestTrack {
        let state = match self.state.as_str() {
            "import_queued" => {
                DownloadState::ImportQueued(self.queue_position.unwrap_or(1).max(1) as u32)
            }
            other => state_from_key(other, self.error.as_deref()),
        };
        DownloadRequestTrack {
            item: self.item_key.clone(),
            source: self.source.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            size: self.size.max(0) as u64,
            quality: self.quality.clone(),
            transferred: self.transferred.max(0) as u64,
            percent: self.percent,
            speed: self.speed,
            state,
            error: self.error.clone(),
            transfer_id: self.transfer_id.clone(),
            local_path: self.local_path.clone(),
            import_output: self.import_output.clone(),
            retryable: self.retryable != 0,
            can_choose_import: self.can_choose_import != 0,
        }
    }

    #[cfg(feature = "server")]
    pub fn to_item(&self) -> DownloadableItem {
        let base = self
            .backend_data
            .as_ref()
            .and_then(|data| serde_json::from_str::<shared::slskd::SearchResult>(data).ok())
            .map(|mut base| {
                base.username.clone_from(&self.source);
                base.filename.clone_from(&self.item_key);
                base.size = self.size.max(0);
                base
            })
            .or(Some(shared::slskd::SearchResult {
                username: self.source.clone(),
                filename: self.item_key.clone(),
                size: self.size.max(0),
                bitrate: None,
                duration: self.duration.map(|d| d.max(0) as i32),
                sample_rate: None,
                bit_depth: None,
                has_free_upload_slot: true,
                upload_speed: 0,
                queue_length: 0,
            }));

        DownloadableItem {
            id: self.item_key.clone(),
            source: self.source.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            size: Some(self.size.max(0) as u64),
            duration: self.duration.map(|d| d.max(0) as u32),
            quality: self.quality.clone(),
            quality_score: self.quality_score,
            backend_data: base.and_then(|base| serde_json::to_string(&base).ok()),
        }
    }
}

impl DownloadRequestRow {
    #[cfg(feature = "server")]
    pub fn to_summary(&self, tracks: Vec<DownloadRequestTrackRow>) -> DownloadRequestSummary {
        DownloadRequestSummary {
            id: self.id.clone(),
            label: self.label.clone(),
            artist: self.artist.clone(),
            target_folder: self.target_folder.clone(),
            state: self.state.clone(),
            tracks: tracks
                .iter()
                .map(DownloadRequestTrackRow::to_shared)
                .collect(),
        }
    }

    #[cfg(feature = "server")]
    pub async fn find_open(user_id: &str, request_key: &str) -> Result<Option<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT * FROM download_requests
             WHERE user_id = ? AND request_key = ? AND state IN ('active', 'stopped')
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(request_key)
        .fetch_optional(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    #[cfg(feature = "server")]
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        id: &str,
        user_id: &str,
        context: &DownloadRequestContext,
        target_folder: &str,
        backend: Option<&str>,
    ) -> Result<Self, String> {
        sqlx::query_as::<_, Self>(
            "INSERT INTO download_requests
                (id, user_id, request_key, label, artist, target_folder, backend)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             RETURNING *",
        )
        .bind(id)
        .bind(user_id)
        .bind(&context.key)
        .bind(&context.label)
        .bind(context.artist.as_deref())
        .bind(target_folder)
        .bind(backend)
        .fetch_one(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    #[cfg(feature = "server")]
    pub async fn get_for_user(id: &str, user_id: &str) -> Result<Option<Self>, String> {
        sqlx::query_as::<_, Self>("SELECT * FROM download_requests WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_optional(&*DB)
            .await
            .map_err(|e| e.to_string())
    }

    #[cfg(feature = "server")]
    pub async fn list_for_user(user_id: &str) -> Result<Vec<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT * FROM download_requests
             WHERE user_id = ? AND state != 'completed'
             ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    #[cfg(feature = "server")]
    pub async fn list_active() -> Result<Vec<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT * FROM download_requests WHERE state = 'active' ORDER BY created_at",
        )
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    #[cfg(feature = "server")]
    pub async fn set_state(id: &str, state: &str) -> Result<(), String> {
        sqlx::query(
            "UPDATE download_requests
             SET state = ?, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
        )
        .bind(state)
        .bind(id)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(feature = "server")]
    pub async fn update_target_folder(id: &str, target_folder: &str) -> Result<(), String> {
        sqlx::query(
            "UPDATE download_requests
             SET target_folder = ?, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
        )
        .bind(target_folder)
        .bind(id)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(feature = "server")]
    pub async fn delete(id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM download_request_tracks WHERE request_id = ?")
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM download_requests WHERE id = ?")
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Mark an active request completed once every track reached a final state.
    #[cfg(feature = "server")]
    pub async fn mark_completed_if_idle(id: &str) -> Result<bool, String> {
        let pending: Option<i64> = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_request_tracks
             WHERE request_id = ?
               AND state NOT IN ('imported', 'import_skipped', 'failed', 'cancelled', 'stopped')",
        )
        .bind(id)
        .fetch_one(&*DB)
        .await
        .map_err(|e| e.to_string())?;

        if pending.unwrap_or(0) > 0 {
            return Ok(false);
        }

        sqlx::query(
            "UPDATE download_requests
             SET state = 'completed', updated_at = CURRENT_TIMESTAMP
             WHERE id = ? AND state = 'active'",
        )
        .bind(id)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// Delete requests that no longer hold any unfinished track.
    #[cfg(feature = "server")]
    pub async fn delete_finished(user_id: &str) -> Result<Vec<String>, String> {
        let rows = sqlx::query_as::<_, Self>(
            "SELECT * FROM download_requests WHERE user_id = ? AND state = 'completed'",
        )
        .bind(user_id)
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())?;

        let mut removed = Vec::new();
        for row in rows {
            let counts: (i64, i64) = sqlx::query_as(
                "SELECT COUNT(*), COALESCE(SUM(CASE WHEN state = 'imported' THEN 1 ELSE 0 END), 0)
                 FROM download_request_tracks WHERE request_id = ?",
            )
            .bind(&row.id)
            .fetch_one(&*DB)
            .await
            .map_err(|e| e.to_string())?;
            if counts.0 > 0 && counts.1 == counts.0 {
                Self::delete(&row.id).await?;
                removed.push(row.id);
            }
        }
        Ok(removed)
    }

    #[cfg(feature = "server")]
    pub async fn delete_failed(user_id: &str) -> Result<Vec<String>, String> {
        let rows = sqlx::query_as::<_, Self>(
            "SELECT * FROM download_requests
             WHERE user_id = ? AND state IN ('completed', 'stopped')",
        )
        .bind(user_id)
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())?;

        let mut removed = Vec::new();
        for row in rows {
            let tracks = DownloadRequestTrackRow::list_for_request(&row.id).await?;
            let has_failed = tracks.iter().any(|track| track.state == "failed");
            let settled = tracks.iter().all(|track| track.state_is_terminal());
            if has_failed && settled {
                Self::delete(&row.id).await?;
                removed.push(row.id);
            }
        }
        Ok(removed)
    }

    #[cfg(feature = "server")]
    pub async fn activity(user_id: &str) -> Result<DownloadActivity, String> {
        #[derive(sqlx::FromRow)]
        struct Row {
            request_label: String,
            request_artist: Option<String>,
            track_title: Option<String>,
            track_artist: Option<String>,
            track_state: Option<String>,
        }

        let rows = sqlx::query_as::<_, Row>(
            "SELECT r.label AS request_label, r.artist AS request_artist,
                    t.title AS track_title, t.artist AS track_artist, t.state AS track_state
             FROM download_requests r
             LEFT JOIN download_request_tracks t ON t.request_id = r.id
             WHERE r.user_id = ? AND r.state IN ('active', 'stopped')
               AND COALESCE(r.backend, '') != 'beets'",
        )
        .bind(user_id)
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())?;

        let mut activity = DownloadActivity::default();
        let mut album_states: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        let mut album_active_counts: std::collections::BTreeMap<String, u32> =
            std::collections::BTreeMap::new();
        for row in rows {
            let track_downloading = row
                .track_state
                .as_deref()
                .map(|state| matches!(state, "in_progress" | "completed" | "importing"))
                .unwrap_or(false);
            let track_pending = row
                .track_state
                .as_deref()
                .map(|state| {
                    !matches!(
                        state,
                        "imported" | "import_skipped" | "failed" | "cancelled" | "stopped"
                    )
                })
                .unwrap_or(false);
            let state = if track_downloading {
                "downloading".to_string()
            } else {
                "requested".to_string()
            };
            let album_key = album_activity_key(
                row.request_artist.as_deref().unwrap_or_default(),
                &row.request_label,
            );
            let priority = |value: &str| if value == "downloading" { 1 } else { 0 };
            album_states
                .entry(album_key.clone())
                .and_modify(|current| {
                    if priority(&state) > priority(current) {
                        *current = state.clone();
                    }
                })
                .or_insert(state.clone());
            if track_pending {
                if let (Some(title), Some(artist)) = (&row.track_title, &row.track_artist) {
                    if track_in_library(user_id, artist, title)
                        .await
                        .unwrap_or(false)
                    {
                        *album_active_counts.entry(album_key.clone()).or_insert(0) += 1;
                    }
                }
            }

            if let (Some(title), Some(artist)) = (row.track_title, row.track_artist) {
                let key = track_activity_key(&artist, &title);
                activity
                    .tracks
                    .entry(key)
                    .and_modify(|current| {
                        if priority(&state) > priority(current) {
                            *current = state.clone();
                        }
                    })
                    .or_insert(state);
            }
        }
        activity.albums = album_states;
        activity.album_active_counts = album_active_counts;

        let user_id_owned = user_id.to_string();
        if let Ok(pending) =
            crate::models::request_queue::LibraryRequestRow::list_for_user(user_id).await
        {
            for request in pending {
                let album_key = album_activity_key(
                    request.artist.as_deref().unwrap_or_default(),
                    &request.label,
                );
                activity
                    .albums
                    .entry(album_key.clone())
                    .or_insert_with(|| "requested".to_string());
                if let Ok(query) =
                    serde_json::from_str::<shared::download::DownloadQuery>(&request.query_json)
                {
                    for track in query.tracks {
                        if track_in_library(&user_id_owned, &track.artist, &track.title)
                            .await
                            .unwrap_or(false)
                        {
                            *activity
                                .album_active_counts
                                .entry(album_key.clone())
                                .or_insert(0) += 1;
                        }
                        activity
                            .tracks
                            .entry(track_activity_key(&track.artist, &track.title))
                            .or_insert_with(|| "requested".to_string());
                    }
                }
            }
        }
        Ok(activity)
    }
}

impl DownloadRequestTrackRow {
    #[cfg(feature = "server")]
    pub async fn list_for_request(request_id: &str) -> Result<Vec<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT * FROM download_request_tracks WHERE request_id = ? ORDER BY id",
        )
        .bind(request_id)
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    #[cfg(feature = "server")]
    pub async fn find_by_item(user_id: &str, item_key: &str) -> Result<Option<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT t.* FROM download_request_tracks t
             JOIN download_requests r ON r.id = t.request_id
             WHERE r.user_id = ? AND t.item_key = ?
             ORDER BY t.id DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(item_key)
        .fetch_optional(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    /// Insert a track if the request does not already hold the same remote file.
    /// Returns true when a row was created.
    #[cfg(feature = "server")]
    pub async fn insert_if_new(request_id: &str, item: &DownloadableItem) -> Result<bool, String> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO download_request_tracks
                (request_id, item_key, source, title, artist, album, size, quality,
                 quality_score, duration, backend_data)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(request_id)
        .bind(&item.id)
        .bind(&item.source)
        .bind(&item.title)
        .bind(&item.artist)
        .bind(&item.album)
        .bind(item.size.unwrap_or(0) as i64)
        .bind(&item.quality)
        .bind(item.quality_score)
        .bind(item.duration.map(|d| d as i64))
        .bind(item.backend_data.as_deref())
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(result.rows_affected() > 0)
    }

    /// Update the row from the request's queued answer.
    #[cfg(feature = "server")]
    pub async fn set_queued(
        request_id: &str,
        item_key: &str,
        transfer_id: &str,
        source: &str,
        item: &str,
        state: &DownloadState,
        error: Option<&str>,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE download_request_tracks
             SET transfer_id = ?, source = ?, item_key = ?, state = ?, error = ?,
                 updated_at = CURRENT_TIMESTAMP
             WHERE request_id = ? AND item_key = ?",
        )
        .bind(transfer_id)
        .bind(source)
        .bind(item)
        .bind(state_key(state))
        .bind(error)
        .bind(request_id)
        .bind(item_key)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Persist a live progress entry keyed by its remote file.
    #[cfg(feature = "server")]
    pub async fn update_progress(
        request_id: &str,
        progress: &DownloadProgress,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE download_request_tracks
             SET transfer_id = ?, state = ?, percent = ?, transferred = ?, speed = ?, error = ?,
                 local_path = ?, import_output = ?, retryable = ?, can_choose_import = ?,
                 queue_position = ?, updated_at = CURRENT_TIMESTAMP
             WHERE request_id = ? AND item_key = ?",
        )
        .bind(&progress.id)
        .bind(state_key(&progress.state))
        .bind(progress.percent)
        .bind(progress.transferred as i64)
        .bind(progress.speed)
        .bind(progress.error.as_deref())
        .bind(progress.local_path.as_deref())
        .bind(progress.import_output.as_deref())
        .bind(i64::from(progress.retryable))
        .bind(i64::from(progress.can_choose_import))
        .bind(queue_position(&progress.state))
        .bind(request_id)
        .bind(&progress.item)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A failover rebinds a track to a new peer and remote path.
    /// Drops the import offers that no stored job backs: flags written by a
    /// process that kept its jobs in memory only.
    #[cfg(feature = "server")]
    pub async fn prune_stale_offers() -> Result<(), String> {
        sqlx::query(
            "UPDATE download_request_tracks
             SET retryable = 0, can_choose_import = 0
             WHERE retryable != 0
               AND NOT EXISTS (
                 SELECT 1
                 FROM import_retries r
                 JOIN download_requests q ON q.id = download_request_tracks.request_id
                 JOIN users u ON u.id = q.user_id
                 WHERE r.username = u.username
                   AND r.item = download_request_tracks.item_key
               )",
        )
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(feature = "server")]
    pub async fn rebind(
        request_id: &str,
        previous_item: &str,
        entry: &DownloadProgress,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE download_request_tracks
             SET item_key = ?, source = ?, transfer_id = ?, state = ?,
                 percent = ?, transferred = ?, speed = ?, error = ?,
                 local_path = ?, import_output = ?, retryable = ?, can_choose_import = ?,
                 queue_position = ?, updated_at = CURRENT_TIMESTAMP
             WHERE request_id = ? AND item_key = ?",
        )
        .bind(&entry.item)
        .bind(&entry.source)
        .bind(&entry.id)
        .bind(state_key(&entry.state))
        .bind(entry.percent)
        .bind(entry.transferred as i64)
        .bind(entry.speed)
        .bind(entry.error.as_deref())
        .bind(entry.local_path.as_deref())
        .bind(entry.import_output.as_deref())
        .bind(i64::from(entry.retryable))
        .bind(i64::from(entry.can_choose_import))
        .bind(queue_position(&entry.state))
        .bind(request_id)
        .bind(previous_item)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(feature = "server")]
    pub async fn mark_stopped(request_id: &str) -> Result<(), String> {
        sqlx::query(
            "UPDATE download_request_tracks
             SET state = 'stopped', updated_at = CURRENT_TIMESTAMP
             WHERE request_id = ?
               AND state NOT IN ('imported', 'import_skipped', 'failed', 'cancelled', 'stopped')",
        )
        .bind(request_id)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(feature = "server")]
    pub async fn mark_failed(request_id: &str, item_key: &str, error: &str) -> Result<(), String> {
        sqlx::query(
            "UPDATE download_request_tracks
             SET state = 'failed', error = ?, updated_at = CURRENT_TIMESTAMP
             WHERE request_id = ? AND item_key = ?",
        )
        .bind(error)
        .bind(request_id)
        .bind(item_key)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(feature = "server")]
    pub async fn set_state(
        request_id: &str,
        item_key: &str,
        state: &DownloadState,
        error: Option<&str>,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE download_request_tracks
             SET state = ?, error = ?, retryable = 0, can_choose_import = 0,
                 queue_position = NULL, updated_at = CURRENT_TIMESTAMP
             WHERE request_id = ? AND item_key = ?",
        )
        .bind(state_key(state))
        .bind(error)
        .bind(request_id)
        .bind(item_key)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_download_state_round_trips_through_storage() {
        let states = [
            DownloadState::Queued,
            DownloadState::InProgress,
            DownloadState::Completed,
            DownloadState::Importing,
            DownloadState::Imported,
            DownloadState::ImportSkipped,
            DownloadState::Cancelled,
            DownloadState::Stopped,
        ];
        for state in states {
            assert_eq!(state_from_key(state_key(&state), None), state);
        }
        assert_eq!(
            state_from_key("failed", Some("peer offline")),
            DownloadState::Failed("peer offline".to_string())
        );
    }

    #[test]
    fn terminal_states_match_the_shared_definition() {
        let keys = [
            "queued",
            "in_progress",
            "completed",
            "importing",
            "imported",
            "import_skipped",
            "failed",
            "cancelled",
            "stopped",
        ];
        for key in keys {
            let state = state_from_key(key, None);
            assert_eq!(state.is_terminal(), state_is_terminal_key(key));
        }
    }
}
