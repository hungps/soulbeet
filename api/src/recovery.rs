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
use crate::models::download_request::{DownloadRequestRow, DownloadRequestTrackRow};
#[cfg(feature = "server")]
use crate::models::folder::Folder;
#[cfg(feature = "server")]
use crate::models::user::User;
#[cfg(feature = "server")]
use crate::server_fns::download::process::process_downloads;
#[cfg(feature = "server")]
use crate::server_fns::download::requests::{broadcast_progress, record_queued, start_monitor};
#[cfg(feature = "server")]
use crate::server_fns::download::utils::resolve_download_path_mapped;
#[cfg(feature = "server")]
use crate::services::download_backend;

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
        .filter(|track| matches!(track.state.as_str(), "completed" | "importing"))
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
        spawn_import(
            import_entries(&request, &importable),
            target.clone(),
            tx.clone(),
            user.id.clone(),
        );
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
                    error: None,
                });
            } else {
                missing.push(item.clone());
            }
            items.push(item);
        }

        if !missing.is_empty() {
            queued.extend(backend.download(missing).await.map_err(|e| e.to_string())?);
        }

        let accepted = record_queued(&request, &items, &queued, &tx).await;
        DownloadRequestRow::set_state(&request.id, "active").await?;
        if !accepted.is_empty() {
            start_monitor(&user.username, &user.id, &request, accepted).await;
        }
        if !importable.is_empty() {
            let request = request.clone();
            let target = target.clone();
            let import_tx = tx.clone();
            let user_id = user.id.clone();
            tokio::spawn(async move {
                import_after_settled(request, target, import_tx, user_id).await;
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
        })
        .collect()
}

#[cfg(feature = "server")]
fn spawn_import(
    entries: Vec<DownloadProgress>,
    target: PathBuf,
    tx: tokio::sync::broadcast::Sender<shared::download::DownloadEvent>,
    user_id: String,
) {
    if entries.is_empty() {
        return;
    }
    tokio::spawn(async move {
        process_downloads(entries, target, tx, user_id).await;
    });
}

#[cfg(feature = "server")]
async fn import_after_settled(
    request: DownloadRequestRow,
    target: PathBuf,
    tx: tokio::sync::broadcast::Sender<shared::download::DownloadEvent>,
    user_id: String,
) {
    for _ in 0..1440 {
        let Ok(tracks) = DownloadRequestTrackRow::list_for_request(&request.id).await else {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        };
        if tracks
            .iter()
            .any(|track| matches!(track.state.as_str(), "queued" | "in_progress" | "importing"))
        {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }
        let importable: Vec<DownloadRequestTrackRow> = tracks
            .into_iter()
            .filter(|track| matches!(track.state.as_str(), "completed" | "importing"))
            .collect();
        if importable.is_empty() {
            return;
        }

        let download_base = crate::models::app_config::AppConfig::download_path().await;
        let slskd_base = crate::models::app_config::AppConfig::slskd_download_path().await;
        let mut entries = Vec::new();
        for track in importable {
            let exists = resolve_download_path_mapped(
                &track.item_key,
                &download_base,
                slskd_base.as_deref(),
            )
            .is_some();
            if exists {
                entries.extend(import_entries(&request, std::slice::from_ref(&track)));
            } else {
                let mut entry = import_entries(&request, std::slice::from_ref(&track))
                    .pop()
                    .expect("entry for track");
                entry.state = DownloadState::Imported;
                let _ = DownloadRequestTrackRow::set_state(
                    &request.id,
                    &track.item_key,
                    &DownloadState::Imported,
                    None,
                )
                .await;
                broadcast_progress(&tx, vec![entry]).await;
            }
        }
        if !entries.is_empty() {
            process_downloads(entries, target, tx, user_id).await;
        }
        return;
    }
}
