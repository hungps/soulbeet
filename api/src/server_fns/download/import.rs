#[cfg(feature = "server")]
use dioxus::logger::tracing::{info, warn};
#[cfg(feature = "server")]
use shared::download::{DownloadEvent, DownloadProgress, DownloadState};
#[cfg(feature = "server")]
use soulbeet::ImportResult;
#[cfg(feature = "server")]
use std::path::Path;
#[cfg(feature = "server")]
use tokio::sync::broadcast;

#[cfg(feature = "server")]
use crate::services::music_importer;

/// Attempt to clean up a failed download/import file
#[cfg(feature = "server")]
async fn cleanup_failed_file(file_path: &str) {
    let path = Path::new(file_path);
    if path.exists() {
        match tokio::fs::remove_file(path).await {
            Ok(_) => info!("Cleaned up failed file: {}", file_path),
            Err(e) => warn!("Failed to clean up file {}: {}", file_path, e),
        }
    }
}

#[cfg(feature = "server")]
pub async fn import_group(
    entries: Vec<DownloadProgress>,
    source_path: String,
    target_path: std::path::PathBuf,
    tx: broadcast::Sender<DownloadEvent>,
    as_album: bool,
    user_id: &str,
) {
    info!(
        "Importing group from: {:?} (album: {})",
        source_path, as_album
    );

    let importing_entries: Vec<_> = entries
        .iter()
        .map(|e| DownloadProgress {
            state: DownloadState::Importing,
            ..e.clone()
        })
        .collect();
    super::requests::broadcast_progress(&tx, importing_entries).await;

    let importer = match music_importer(None).await {
        Ok(imp) => imp,
        Err(e) => {
            warn!("Failed to get importer: {}", e);
            let failed_entries: Vec<_> = entries
                .iter()
                .map(|entry| DownloadProgress {
                    state: DownloadState::Failed(format!("No importer available: {e}")),
                    error: Some(format!("No importer available: {e}")),
                    ..entry.clone()
                })
                .collect();
            super::requests::broadcast_progress(&tx, failed_entries).await;
            return;
        }
    };

    let source = Path::new(&source_path);
    match importer.import(&[source], &target_path, as_album).await {
        Ok(ImportResult::Success) => {
            info!("Import successful");
            let imported_entries: Vec<_> = entries
                .iter()
                .map(|e| DownloadProgress {
                    state: DownloadState::Imported,
                    ..e.clone()
                })
                .collect();
            super::requests::broadcast_progress(&tx, imported_entries).await;

            let user_id = user_id.to_string();
            let scan_target = target_path.clone();
            tokio::spawn(async move {
                crate::server_fns::media_server::trigger_media_scan(&user_id, &scan_target).await;
            });

            let library_target = target_path.clone();
            tokio::spawn(async move {
                crate::library::scan_folder_containing(&library_target).await;
            });
        }
        Ok(ImportResult::Skipped(reason)) => {
            info!("Import skipped items: {}", reason);
            let skipped_entries: Vec<_> = entries
                .iter()
                .map(|e| DownloadProgress {
                    state: DownloadState::ImportSkipped,
                    // Carry the reason to the UI: a skipped row that explains
                    // nothing is the whole of issue #69.
                    error: Some(reason.clone()),
                    ..e.clone()
                })
                .collect();
            super::requests::broadcast_progress(&tx, skipped_entries).await;

            for entry in &entries {
                cleanup_failed_file(&entry.item).await;
            }
        }
        Ok(ImportResult::Failed(err)) => {
            info!("Import failed: {}", err);
            let failed_entries: Vec<_> = entries
                .iter()
                .map(|e| DownloadProgress {
                    state: DownloadState::Failed(format!("Import failed: {err}")),
                    error: Some(format!("Import failed: {err}")),
                    ..e.clone()
                })
                .collect();
            super::requests::broadcast_progress(&tx, failed_entries).await;

            for entry in &entries {
                cleanup_failed_file(&entry.item).await;
            }
        }
        Ok(ImportResult::TimedOut) => {
            warn!("Import timed out for: {}", source_path);
            let failed_entries: Vec<_> = entries
                .iter()
                .map(|e| DownloadProgress {
                    state: DownloadState::Failed("Import timed out".into()),
                    error: Some("Import timed out".into()),
                    ..e.clone()
                })
                .collect();
            super::requests::broadcast_progress(&tx, failed_entries).await;

            for entry in &entries {
                cleanup_failed_file(&entry.item).await;
            }
        }
        Err(e) => {
            warn!("Import error for {}: {}", source_path, e);
            let failed_entries: Vec<_> = entries
                .iter()
                .map(|entry| DownloadProgress {
                    state: DownloadState::Failed(format!("Import error: {e}")),
                    error: Some(format!("Import error: {e}")),
                    ..entry.clone()
                })
                .collect();
            super::requests::broadcast_progress(&tx, failed_entries).await;

            for entry in &entries {
                cleanup_failed_file(&entry.item).await;
            }
        }
    }

    // beets moves the files out but never prunes the folder they came from:
    // the import path is that folder (album) or the file itself (singleton).
    let source_dir = if as_album {
        Some(source)
    } else {
        source.parent()
    };
    if let Some(dir) = source_dir {
        let download_base = crate::models::app_config::AppConfig::download_path().await;
        let _ = crate::server_fns::cleanup_empty_ancestors(dir, &download_base).await;
    }
}
