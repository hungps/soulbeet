#[cfg(feature = "server")]
use std::path::PathBuf;
#[cfg(feature = "server")]
use std::sync::Once;

#[cfg(feature = "server")]
use dioxus::logger::tracing::{info, warn};
#[cfg(feature = "server")]
use shared::download::{DownloadProgress, DownloadState, QueuedDownload};

#[cfg(feature = "server")]
use crate::globals::get_or_create_user_channel;
#[cfg(feature = "server")]
use crate::import_queue::IMPORT_QUEUE;
#[cfg(feature = "server")]
use crate::models::download_request::{DownloadRequestRow, DownloadRequestTrackRow};
#[cfg(feature = "server")]
use crate::models::folder::Folder;
#[cfg(feature = "server")]
use crate::models::user::User;
use crate::server_fns::download::enqueue::transfer_key;
use crate::server_fns::download::process::import_job as make_import_job;
#[cfg(feature = "server")]
use crate::server_fns::download::requests::{broadcast_progress, record_queued, start_monitor};
#[cfg(feature = "server")]
use crate::server_fns::download::utils::resolve_download_path_mapped;
#[cfg(feature = "server")]
use crate::services::download_backend;
use crate::transfer_registry::TransferRegistry;

#[cfg(feature = "server")]
static RECOVERY_STARTED: Once = Once::new();

pub fn start_startup_recovery() {
    #[cfg(feature = "server")]
    RECOVERY_STARTED.call_once(|| {
        tokio::spawn(async {
            run_startup_recovery().await;
        });
    });
}

#[cfg(feature = "server")]
async fn run_startup_recovery() {
    info!("Startup recovery: rescanning library folders");

    if let Err(e) = DownloadRequestTrackRow::reset_kept_offers().await {
        warn!("Startup recovery: could not reset kept import offers: {e}");
    }

    let folders = Folder::get_all().await.unwrap_or_default();
    tokio::spawn(async move {
        for folder in folders {
            if let Err(e) = crate::library::scan_folder_internal(&folder.id, true).await {
                warn!(
                    "Startup recovery: library scan failed for '{}': {}",
                    folder.name, e
                );
            }
        }
    });

    let requests = match DownloadRequestRow::list_active().await {
        Ok(requests) => requests,
        Err(e) => {
            warn!("Startup recovery: could not list active requests: {}", e);
            return;
        }
    };

    if requests.is_empty() {
        info!("Startup recovery: no active download requests");
        return;
    }

    let importer_issue = match crate::services::music_importer(None).await {
        Ok(importer) => importer
            .refresh_health()
            .await
            .blocking_issue()
            .map(|issue| issue.title.clone()),
        Err(e) => Some(format!("no importer available: {e}")),
    };
    if let Some(issue) = importer_issue {
        warn!(
            "Startup recovery: beets cannot import ({issue}); leaving {} request(s) stopped",
            requests.len()
        );
        for request in requests {
            let _ = DownloadRequestRow::set_state(&request.id, "stopped").await;
        }
        return;
    }

    info!(
        "Startup recovery: resuming {} active download request(s)",
        requests.len()
    );
    for request in requests {
        let request_id = request.id.clone();
        tokio::spawn(async move {
            if let Err(e) = recover_request(request).await {
                warn!("Startup recovery: request recovery failed: {}", e);
                let _ = DownloadRequestRow::set_state(&request_id, "stopped").await;
            }
        });
    }
}

#[cfg(feature = "server")]
async fn recover_request(request: DownloadRequestRow) -> Result<(), String> {
    let user = User::get_by_id(&request.user_id).await?;
    let tracks = DownloadRequestTrackRow::list_for_request(&request.id).await?;

    let importable: Vec<DownloadRequestTrackRow> = tracks
        .iter()
        .filter(|track| {
            matches!(
                track.state.as_str(),
                "completed" | "importing" | "import_queued"
            )
        })
        .cloned()
        .collect();
    let resumable: Vec<DownloadRequestTrackRow> = tracks
        .into_iter()
        .filter(|track| {
            matches!(
                track.state.as_str(),
                "queued" | "in_progress" | "stopped" | "cancelled"
            )
        })
        .collect();

    let (tx, _) = get_or_create_user_channel(&user.username).await;

    let target = PathBuf::from(&request.target_folder);

    if !importable.is_empty() && resumable.is_empty() {
        import_tracks(&request, &importable, &target, &user.username, &tx).await;
    }

    if !resumable.is_empty() {
        let backend = download_backend(request.backend.as_deref()).await?;
        let sources: Vec<String> = resumable.iter().map(|track| track.source.clone()).collect();
        let existing = backend
            .get_downloads_for(&sources)
            .await
            .unwrap_or_default();
        let mut items = Vec::new();
        let mut queued: Vec<QueuedDownload> = Vec::new();
        let mut missing = Vec::new();

        for track in &resumable {
            let item = track.to_item();
            if let Some(download) = existing
                .iter()
                .find(|download| download.source == track.source && download.item == track.item_key)
            {
                queued.push(QueuedDownload {
                    id: download.id.clone(),
                    source: download.source.clone(),
                    item: download.item.clone(),
                    size: download.size,
                    status: shared::download::EnqueueStatus::Enqueued,
                });
            } else {
                missing.push(item.clone());
            }
            items.push(item);
        }

        if !missing.is_empty() {
            let batch = crate::server_fns::download::requests::batch_for(&request);
            let keys: Vec<_> = missing
                .iter()
                .map(|item| transfer_key(backend.id(), &item.source, &item.id))
                .collect();
            let claim = TransferRegistry::global().claim(&keys, &batch);
            let free: Vec<_> = missing
                .into_iter()
                .filter(|item| {
                    claim
                        .claimed
                        .iter()
                        .any(|key| key.peer == item.source && key.filename == item.id)
                })
                .collect();
            if !free.is_empty() {
                let destination = shared::download::batch_folder(&request.label, &request.id);
                queued.extend(
                    backend
                        .download(free, &destination)
                        .await
                        .map_err(|e| e.to_string())?,
                );
            }
        }

        let accepted = record_queued(&request, &items, &queued, &tx).await;
        DownloadRequestRow::set_state(&request.id, "active").await?;
        if !accepted.is_empty() {
            start_monitor(&user.username, &request, accepted).await;
        }
        if !importable.is_empty() {
            let request = request.clone();
            let target = target.clone();
            let import_tx = tx.clone();
            let username = user.username.clone();
            tokio::spawn(async move {
                import_after_settled(request, target, import_tx, username).await;
            });
        }
    }

    let _ = DownloadRequestRow::mark_completed_if_idle(&request.id).await;

    Ok(())
}

#[cfg(feature = "server")]
fn import_entries(
    request: &DownloadRequestRow,
    tracks: &[DownloadRequestTrackRow],
) -> Vec<DownloadProgress> {
    tracks
        .iter()
        .map(|track| DownloadProgress {
            id: track
                .transfer_id
                .clone()
                .unwrap_or_else(|| track.item_key.clone()),
            source: track.source.clone(),
            item: track.item_key.clone(),
            size: track.size.max(0) as u64,
            transferred: track.size.max(0) as u64,
            state: DownloadState::Completed,
            percent: 100.0,
            speed: 0.0,
            error: None,
            backend: request.backend.clone(),
            batch_id: Some(request.id.clone()),
            batch_label: Some(request.label.clone()),
            local_path: None,
            import_output: None,
            retryable: false,
            can_choose_import: false,
        })
        .collect()
}

#[cfg(feature = "server")]
async fn import_tracks(
    request: &DownloadRequestRow,
    tracks: &[DownloadRequestTrackRow],
    target: &std::path::Path,
    username: &str,
    tx: &tokio::sync::broadcast::Sender<shared::download::DownloadEvent>,
) {
    if tracks.is_empty() {
        return;
    }
    let download_base = crate::models::app_config::AppConfig::download_path().await;
    let slskd_base = crate::models::app_config::AppConfig::slskd_download_path().await;
    let album_mode = crate::models::app_config::AppConfig::is_album_mode().await;

    let mut groups: Vec<(PathBuf, Vec<DownloadProgress>)> = Vec::new();
    for track in tracks {
        let entry = import_entries(request, std::slice::from_ref(track))
            .pop()
            .expect("entry for track");
        let Some(path) =
            resolve_download_path_mapped(&track.item_key, &download_base, slskd_base.as_deref())
        else {
            let mut gone = entry;
            gone.state = DownloadState::Imported;
            let _ = DownloadRequestTrackRow::set_state(
                &request.id,
                &track.item_key,
                &DownloadState::Imported,
                None,
            )
            .await;
            broadcast_progress(tx, vec![gone]).await;
            continue;
        };
        let path = PathBuf::from(path);
        let source = match album_mode {
            true => path
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or(path),
            false => path,
        };
        match groups.iter_mut().find(|(dir, _)| *dir == source) {
            Some((_, rows)) => rows.push(entry),
            None => groups.push((source, vec![entry])),
        }
    }

    for (source, rows) in groups {
        let mode = match album_mode {
            true => soulbeet::ImportMode::Album,
            false => soulbeet::ImportMode::Singleton,
        };
        let job = make_import_job(source, mode, rows, target, username, &download_base);
        IMPORT_QUEUE.submit(job).await;
    }
}

#[cfg(feature = "server")]
async fn import_after_settled(
    request: DownloadRequestRow,
    target: PathBuf,
    tx: tokio::sync::broadcast::Sender<shared::download::DownloadEvent>,
    username: String,
) {
    for _ in 0..1440 {
        let Ok(tracks) = DownloadRequestTrackRow::list_for_request(&request.id).await else {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        };
        if tracks.iter().any(|track| {
            matches!(
                track.state.as_str(),
                "queued" | "in_progress" | "importing" | "import_queued"
            )
        }) {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }
        let importable: Vec<DownloadRequestTrackRow> = tracks
            .into_iter()
            .filter(|track| {
                matches!(
                    track.state.as_str(),
                    "completed" | "importing" | "import_queued"
                )
            })
            .collect();
        if importable.is_empty() {
            return;
        }
        import_tracks(&request, &importable, &target, &username, &tx).await;
        return;
    }
}
