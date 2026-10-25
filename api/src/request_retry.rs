#[cfg(feature = "server")]
use std::time::Duration;

#[cfg(feature = "server")]
use dioxus::logger::tracing::{info, warn};
#[cfg(feature = "server")]
use shared::download::{DownloadEvent, DownloadableGroup, SearchState};
#[cfg(feature = "server")]
use uuid::Uuid;

#[cfg(feature = "server")]
use crate::globals::get_or_create_user_channel;
#[cfg(feature = "server")]
use crate::models::download_request::{DownloadRequestRow, DownloadRequestTrackRow};
#[cfg(feature = "server")]
use crate::models::request_queue::LibraryRequestRow;
#[cfg(feature = "server")]
use crate::models::user::User;
#[cfg(feature = "server")]
use crate::services::{available_download_backends, download_backend};

#[cfg(feature = "server")]
const SEARCH_TIMEOUT: Duration = Duration::from_secs(120);
#[cfg(feature = "server")]
const SEARCH_POLL_INTERVAL: Duration = Duration::from_secs(2);

pub fn start_request_retry_task() {
    #[cfg(feature = "server")]
    tokio::spawn(async {
        loop {
            if let Err(e) = retry_due_requests().await {
                info!("Request retry pass failed: {}", e);
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

/// Parked requests wait while beets cannot import, so a broken importer never
/// turns into downloads nobody can file.
#[cfg(feature = "server")]
async fn importer_ready() -> bool {
    match crate::importer_health::not_ready().await {
        Some(issue) => {
            info!("Request retry: beets cannot import ({issue}), keeping parked requests");
            false
        }
        None => true,
    }
}

#[cfg(feature = "server")]
fn due(last_attempt_at: Option<&str>, interval: Duration) -> bool {
    let Some(last) = last_attempt_at else {
        return true;
    };
    let parsed = chrono::DateTime::parse_from_rfc3339(last)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(last, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|value| value.and_utc())
        });
    let Some(parsed) = parsed else {
        return true;
    };
    chrono::Utc::now()
        .signed_duration_since(parsed)
        .to_std()
        .map(|elapsed| elapsed >= interval)
        .unwrap_or(true)
}

#[cfg(feature = "server")]
async fn retry_due_requests() -> Result<(), String> {
    let settings = crate::library::scan_settings().await;
    let Some(interval) = settings.request_retry_interval.duration() else {
        return Ok(());
    };
    if !importer_ready().await {
        return Ok(());
    }

    for row in LibraryRequestRow::list_all().await? {
        if !due(row.last_attempt_at.as_deref(), interval) {
            continue;
        }
        if let Err(e) = retry_one(&row).await {
            warn!("Request retry failed for '{}': {}", row.label, e);
            let _ = LibraryRequestRow::mark_attempt(&row.id).await;
        }
    }
    Ok(())
}

#[cfg(feature = "server")]
async fn retry_one(row: &LibraryRequestRow) -> Result<(), String> {
    if row.attempts >= 20 {
        let _ = LibraryRequestRow::delete(&row.id).await;
        return Ok(());
    }
    let user = User::get_by_id(&row.user_id).await?;
    let folder = crate::models::folder::Folder::get_by_id(&row.folder_id).await?;
    let Some(folder) = folder else {
        let _ = LibraryRequestRow::delete(&row.id).await;
        return Ok(());
    };
    let mut query = row.query()?;
    crate::server_fns::search::hydrate_album_tracks(&mut query).await?;

    let backend_ids: Vec<String> = available_download_backends()
        .iter()
        .map(|(id, _)| id.to_string())
        .collect();
    let mut backends = Vec::new();
    for id in &backend_ids {
        if let Ok(backend) = download_backend(Some(id)).await {
            backends.push(backend);
        }
    }
    if backends.is_empty() {
        return Err("no download backend available".to_string());
    }

    let album = query.album.clone();
    let tracks = query.tracks.clone();
    let searches = backends.iter().map(|backend| {
        let album = album.clone();
        let tracks = tracks.clone();
        async move {
            let Ok(search_id) = backend.start_search(album.as_ref(), &tracks).await else {
                return Vec::<DownloadableGroup>::new();
            };
            let deadline = tokio::time::Instant::now() + SEARCH_TIMEOUT;
            let mut latest = Vec::new();
            loop {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(SEARCH_POLL_INTERVAL).await;
                match backend.poll_search(&search_id).await {
                    Ok(result) => {
                        if !result.groups.is_empty() {
                            latest = result.groups;
                        }
                        if result.state != SearchState::InProgress {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            latest
        }
    });
    let mut groups: Vec<DownloadableGroup> = futures::future::join_all(searches)
        .await
        .into_iter()
        .flatten()
        .collect();
    groups.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let pick = groups
        .iter()
        .position(shared::download::DownloadableGroup::auto_pickable);
    let Some(pick) = pick else {
        LibraryRequestRow::mark_attempt(&row.id).await?;
        return Ok(());
    };

    let picked = groups.remove(pick);
    if picked.items.is_empty() {
        LibraryRequestRow::mark_attempt(&row.id).await?;
        return Ok(());
    }

    let target = std::path::PathBuf::from(&folder.path);
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| format!("Failed to create target directory: {e}"))?;

    let batch_id = Uuid::new_v4().to_string();
    let request_context = query.request_context();
    let request = DownloadRequestRow::create(
        &batch_id,
        &row.user_id,
        &request_context,
        &folder.path,
        None,
    )
    .await?;
    for item in &picked.items {
        let _ = DownloadRequestTrackRow::insert_if_new(&request.id, item).await;
    }

    let (tx, _) = get_or_create_user_channel(&user.username).await;
    let accepted = match crate::server_fns::download::requests::enqueue_for_request(
        &user.username,
        &request,
        picked.items.clone(),
        &tx,
    )
    .await
    {
        Ok(accepted) => accepted,
        Err(e) => {
            let _ = DownloadRequestRow::delete(&request.id).await;
            return Err(format!("queue failed: {e}"));
        }
    };

    if accepted.is_empty() {
        let _ = DownloadRequestRow::delete(&request.id).await;
        LibraryRequestRow::mark_attempt(&row.id).await?;
        return Ok(());
    }
    let _ = tx.send(DownloadEvent::AutoDownload(
        shared::download::AutoDownloadEvent::Downloading {
            batch_id: request.id.clone(),
        },
    ));
    LibraryRequestRow::delete(&row.id).await?;
    info!(
        "Retried request '{}' for {}: downloading from {}",
        row.label, user.username, picked.source
    );
    Ok(())
}
