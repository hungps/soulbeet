//! Request sessions: one card in the transfer panel per release. A manual
//! download creates a request on first use and later downloads for the same
//! release attach to it.

use serde::{Deserialize, Serialize};
use shared::download::{DownloadRequestSummary, PendingRequestSummary};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestAction {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAction {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequestsOverview {
    pub requests: Vec<DownloadRequestSummary>,
    pub pending: Vec<PendingRequestSummary>,
}

#[cfg(feature = "server")]
use std::collections::HashMap;
#[cfg(feature = "server")]
use std::path::PathBuf;
#[cfg(feature = "server")]
use std::sync::LazyLock;

#[cfg(feature = "server")]
use dioxus::logger::tracing::{info, warn};
use dioxus::prelude::*;
#[cfg(feature = "server")]
use shared::download::{
    DownloadEvent, DownloadProgress, DownloadRequestContext, DownloadState, DownloadableItem,
    EnqueueStatus, QueuedDownload,
};
#[cfg(feature = "server")]
use tokio::sync::{broadcast, RwLock};
#[cfg(feature = "server")]
use tokio_util::sync::CancellationToken;

#[cfg(feature = "server")]
use crate::globals::{get_or_create_user_channel, register_user_task, unregister_user_task};
#[cfg(feature = "server")]
use crate::models::download_request::{
    state_is_terminal_key, DownloadRequestRow, DownloadRequestTrackRow,
};
#[cfg(feature = "server")]
use crate::models::request_queue::LibraryRequestRow;
#[cfg(feature = "server")]
use crate::server_fns::server_error;
#[cfg(feature = "server")]
use crate::services::download_backend;
#[cfg(feature = "server")]
use crate::AuthSession;

#[cfg(feature = "server")]
use super::enqueue::{
    deliver, enqueue_request, failed_reason, stopped_rows, transfer_key, RequestEnqueue, Watcher,
};
#[cfg(feature = "server")]
use super::monitor::{DownloadMonitor, WatchedTransfer};
#[cfg(feature = "server")]
use super::utils::resolve_download_path_mapped;
#[cfg(feature = "server")]
use crate::transfer_registry::{BatchRef, TransferOwner};

#[cfg(feature = "server")]
static REQUEST_TOKENS: LazyLock<RwLock<HashMap<String, CancellationToken>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(feature = "server")]
pub(crate) async fn request_token(username: &str, request_id: &str) -> CancellationToken {
    let mut tokens = REQUEST_TOKENS.write().await;
    if let Some(token) = tokens.get(request_id) {
        if !token.is_cancelled() {
            return token.clone();
        }
    }
    let (_, user_token) = get_or_create_user_channel(username).await;
    let token = user_token.child_token();
    tokens.insert(request_id.to_string(), token.clone());
    token
}

#[cfg(feature = "server")]
pub async fn cancel_request_token(request_id: &str) {
    let tokens = REQUEST_TOKENS.read().await;
    if let Some(token) = tokens.get(request_id) {
        token.cancel();
    }
}

#[cfg(feature = "server")]
pub async fn forget_request_token(request_id: &str) {
    REQUEST_TOKENS.write().await.remove(request_id);
}

/// Broadcast progress and persist the rows the event belongs to.
#[cfg(feature = "server")]
pub async fn broadcast_progress(
    tx: &broadcast::Sender<DownloadEvent>,
    entries: Vec<DownloadProgress>,
) {
    let mut touched: Vec<String> = Vec::new();
    for entry in &entries {
        if let Some(request_id) = &entry.batch_id {
            let _ = DownloadRequestTrackRow::update_progress(request_id, entry).await;
            if !touched.contains(request_id) {
                touched.push(request_id.clone());
            }
        }
    }
    for request_id in touched {
        if let Ok(true) = DownloadRequestRow::mark_completed_if_idle(&request_id).await {
            forget_request_token(&request_id).await;
        }
    }
    let _ = tx.send(DownloadEvent::Progress(entries));
}

#[cfg(feature = "server")]
pub async fn open_request(
    user_id: &str,
    context: &DownloadRequestContext,
    target_folder: &str,
    backend: Option<&str>,
) -> Result<DownloadRequestRow, String> {
    if let Some(existing) = DownloadRequestRow::find_open(user_id, &context.key).await? {
        if existing.target_folder != target_folder {
            DownloadRequestRow::update_target_folder(&existing.id, target_folder).await?;
        }
        if existing.context_json.is_none() {
            let _ = DownloadRequestRow::update_context(&existing.id, context).await;
        }
        DownloadRequestRow::set_state(&existing.id, "active").await?;
        return Ok(existing);
    }

    let id = uuid::Uuid::new_v4().to_string();
    DownloadRequestRow::create(id.as_str(), user_id, context, target_folder, backend).await
}

/// Insert the items that are not already part of the request. Returns the new
/// items, i.e. the only ones that may be enqueued at the backend.
#[cfg(feature = "server")]
pub async fn insert_new_items(
    request: &DownloadRequestRow,
    items: Vec<DownloadableItem>,
) -> Result<Vec<DownloadableItem>, String> {
    let mut inserted = Vec::new();
    for item in items {
        if DownloadRequestTrackRow::insert_if_new(&request.id, &item).await? {
            inserted.push(item);
        }
    }
    Ok(inserted)
}

/// Persist the backend's enqueue answers and broadcast their initial state.
/// Returns the accepted downloads, in the order they were enqueued.
#[cfg(feature = "server")]
pub async fn record_queued(
    request: &DownloadRequestRow,
    items: &[DownloadableItem],
    queued: &[QueuedDownload],
    tx: &broadcast::Sender<DownloadEvent>,
) -> Vec<QueuedDownload> {
    let mut accepted = Vec::new();
    let mut entries = Vec::new();

    for item in items {
        let answer = queued.iter().find(|q| q.item == item.id || q.id == item.id);

        match answer {
            Some(q) if q.status == EnqueueStatus::Enqueued => {
                let _ = DownloadRequestTrackRow::set_queued(
                    &request.id,
                    &item.id,
                    &q.id,
                    &q.source,
                    &q.item,
                    &DownloadState::Queued,
                    None,
                )
                .await;
                let mut entry = DownloadProgress::queued(
                    q.id.clone(),
                    q.source.clone(),
                    q.item.clone(),
                    q.size,
                );
                entry.backend = request.backend.clone();
                entries.push(entry.with_batch(request.id.clone(), request.label.clone()));
                accepted.push(q.clone());
            }
            Some(q) => match failed_reason(&q.status) {
                Some(error) => {
                    let error = error.to_string();
                    let _ =
                        DownloadRequestTrackRow::mark_failed(&request.id, &item.id, &error).await;
                    let mut entry = DownloadProgress::failed(
                        q.id.clone(),
                        q.source.clone(),
                        q.item.clone(),
                        error,
                    );
                    entry.backend = request.backend.clone();
                    entries.push(entry.with_batch(request.id.clone(), request.label.clone()));
                }
                None => {
                    let _ = DownloadRequestTrackRow::set_queued(
                        &request.id,
                        &item.id,
                        &q.id,
                        &q.source,
                        &q.item,
                        &DownloadState::Queued,
                        None,
                    )
                    .await;
                    let mut entry = DownloadProgress::queued(
                        q.id.clone(),
                        q.source.clone(),
                        q.item.clone(),
                        q.size,
                    );
                    entry.backend = request.backend.clone();
                    entries.push(entry.with_batch(request.id.clone(), request.label.clone()));
                }
            },
            None => {
                let error = "Not reported in the backend enqueue response".to_string();
                let _ = DownloadRequestTrackRow::mark_failed(&request.id, &item.id, &error).await;
                let mut entry = DownloadProgress::failed(
                    item.id.clone(),
                    item.source.clone(),
                    item.id.clone(),
                    error,
                );
                entry.backend = request.backend.clone();
                entries.push(entry.with_batch(request.id.clone(), request.label.clone()));
            }
        }
    }

    if !entries.is_empty() {
        broadcast_progress(tx, entries).await;
    }

    accepted
}

#[cfg(feature = "server")]
pub fn batch_for(request: &DownloadRequestRow) -> BatchRef {
    BatchRef::new(
        TransferOwner {
            user_id: request.user_id.clone(),
            batch_id: request.id.clone(),
        },
        request.label.clone(),
        PathBuf::from(&request.target_folder),
    )
}

#[cfg(feature = "server")]
pub async fn start_monitor(
    username: &str,
    request: &DownloadRequestRow,
    accepted: Vec<QueuedDownload>,
) {
    if accepted.is_empty() {
        return;
    }
    let batch = batch_for(request);
    if let Ok(backend) = download_backend(request.backend.as_deref()).await {
        let keys: Vec<crate::transfer_registry::TransferKey> = accepted
            .iter()
            .map(|q| transfer_key(backend.id(), &q.source, &q.item))
            .collect();
        crate::transfer_registry::TransferRegistry::global().claim(&keys, &batch);
    }
    let (tx, _) = get_or_create_user_channel(username).await;
    start_monitor_on_batch(username, request, accepted, batch, tx).await;
}

#[cfg(feature = "server")]
pub async fn spawn_monitor(
    username: &str,
    accepted: Vec<QueuedDownload>,
    batch: BatchRef,
    tx: broadcast::Sender<DownloadEvent>,
) {
    if accepted.is_empty() {
        return;
    }
    let token = register_user_task(username).await;
    let username = username.to_string();
    let album_mode = crate::models::app_config::AppConfig::is_album_mode().await;
    let download_base = crate::models::app_config::AppConfig::download_path().await;
    tokio::spawn(async move {
        let mut monitor = DownloadMonitor::new(
            accepted.iter().map(WatchedTransfer::from).collect(),
            batch.target.clone(),
            tx,
            token,
            username.clone(),
            batch,
            album_mode,
            download_base,
        );
        monitor.run().await;
        unregister_user_task(&username).await;
    });
}

#[cfg(feature = "server")]
pub async fn start_monitor_on_batch(
    username: &str,
    request: &DownloadRequestRow,
    accepted: Vec<QueuedDownload>,
    batch: BatchRef,
    tx: broadcast::Sender<DownloadEvent>,
) {
    if accepted.is_empty() {
        return;
    }
    let _user_token = register_user_task(username).await;
    let token = request_token(username, &request.id).await;
    let username = username.to_string();
    let album_mode = crate::models::app_config::AppConfig::is_album_mode().await;
    let download_base = crate::models::app_config::AppConfig::download_path().await;
    tokio::spawn(async move {
        let mut monitor = DownloadMonitor::new(
            accepted.iter().map(WatchedTransfer::from).collect(),
            batch.target.clone(),
            tx,
            token,
            username.clone(),
            batch,
            album_mode,
            download_base,
        );
        monitor.run().await;
        unregister_user_task(&username).await;
    });
}

#[cfg(feature = "server")]
fn progress_from_track(
    request: &DownloadRequestRow,
    track: &DownloadRequestTrackRow,
    state: DownloadState,
    error: Option<String>,
) -> DownloadProgress {
    let mut entry = DownloadProgress {
        id: track.transfer_id.clone().unwrap_or_default(),
        source: track.source.clone(),
        item: track.item_key.clone(),
        size: track.size.max(0) as u64,
        transferred: track.transferred.max(0) as u64,
        state,
        percent: track.percent,
        speed: track.speed,
        error,
        backend: request.backend.clone(),
        batch_id: Some(request.id.clone()),
        batch_label: Some(request.label.clone()),
        local_path: None,
        import_output: None,
        retryable: false,
        can_choose_import: false,
    };
    if entry.id.is_empty() {
        entry.id = track.item_key.clone();
    }
    entry
}

#[cfg(feature = "server")]
async fn owned_request(id: &str, user_id: &str) -> Result<DownloadRequestRow, ServerFnError> {
    DownloadRequestRow::get_for_user(id, user_id)
        .await
        .map_err(server_error)?
        .ok_or_else(|| server_error("Download request not found"))
}

#[cfg(feature = "server")]
async fn cancel_tracks(
    request: &DownloadRequestRow,
    tracks: &[DownloadRequestTrackRow],
    remove: bool,
) -> Result<(), String> {
    let backend = download_backend(request.backend.as_deref())
        .await
        .map_err(|e| format!("download backend not available: {e}"))?;

    let sources: Vec<String> = tracks
        .iter()
        .filter(|t| !t.state_is_terminal())
        .map(|t| t.source.clone())
        .collect();
    let transfers = if sources.is_empty() {
        Vec::new()
    } else {
        backend
            .get_downloads_for(&sources)
            .await
            .unwrap_or_default()
    };

    for track in tracks.iter().filter(|t| !t.state_is_terminal()) {
        let id = track
            .transfer_id
            .as_ref()
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .cloned()
            .or_else(|| {
                transfers
                    .iter()
                    .find(|dl| dl.source == track.source && dl.item == track.item_key)
                    .map(|dl| dl.id.clone())
            });
        let Some(id) = id else {
            continue;
        };
        if let Err(e) = backend.cancel_download(&track.source, &id, remove).await {
            warn!(
                "Could not cancel transfer {} for request {}: {}",
                id, request.id, e
            );
        }
    }
    Ok(())
}

/// Enqueues `items` for `request` through the transfer registry and starts
/// (or joins) the monitor that watches them. Rows are persisted and sent to
/// the UI on the way.
#[cfg(feature = "server")]
pub async fn enqueue_for_request(
    username: &str,
    request: &DownloadRequestRow,
    items: Vec<DownloadableItem>,
    tx: &broadcast::Sender<DownloadEvent>,
) -> Result<Vec<QueuedDownload>, String> {
    let backend = download_backend(request.backend.as_deref())
        .await
        .map_err(|e| format!("download backend not available: {e}"))?;
    let enq = enqueue_request(&backend, items.clone(), batch_for(request)).await;
    let accepted = record_queued(request, &items, &enq.results, tx).await;
    let RequestEnqueue {
        batch,
        watcher,
        error,
        ..
    } = enq;
    if let Some(error) = error {
        return Err(format!("download queue failed: {error}"));
    }
    match watcher {
        Watcher::Joined(reservation) => {
            let lost = deliver(reservation, &accepted, &backend, &batch).await;
            if !lost.is_empty() {
                let _ = tx.send(DownloadEvent::Progress(stopped_rows(&lost, &batch)));
            }
        }
        Watcher::Own if accepted.is_empty() && batch.inbox.close_if_idle() => {}
        Watcher::Own => {
            start_monitor_on_batch(username, request, accepted.clone(), batch, tx.clone()).await;
        }
    }
    Ok(accepted)
}

#[cfg(feature = "server")]
pub(crate) async fn requeue(
    username: &str,
    request: &DownloadRequestRow,
    tracks: Vec<DownloadRequestTrackRow>,
    tx: &broadcast::Sender<DownloadEvent>,
) -> Result<Vec<QueuedDownload>, String> {
    let items: Vec<DownloadableItem> = tracks.iter().map(|track| track.to_item()).collect();
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let accepted = enqueue_for_request(username, request, items, tx).await?;
    DownloadRequestRow::set_state(&request.id, "active").await?;
    Ok(accepted)
}

#[get("/api/downloads/requests", auth: AuthSession)]
pub async fn list_download_requests() -> Result<DownloadRequestsOverview, ServerFnError> {
    let requests = DownloadRequestRow::list_for_user(&auth.0.sub)
        .await
        .map_err(server_error)?;
    let mut summaries = Vec::with_capacity(requests.len());
    for request in requests {
        let tracks = DownloadRequestTrackRow::list_for_request(&request.id)
            .await
            .map_err(server_error)?;
        summaries.push(request.to_summary(tracks));
    }

    let mut pending = crate::globals::pending_requests_for(&auth.0.username);
    if let Ok(queued) = LibraryRequestRow::list_for_user(&auth.0.sub).await {
        pending.extend(queued.into_iter().map(|row| {
            let track_count = row.query().map(|query| query.tracks.len()).unwrap_or(0);
            PendingRequestSummary {
                id: row.id,
                label: row.label,
                artist: row.artist,
                state: "requested".to_string(),
                track_count,
                attempts: row.attempts.max(0) as u32,
            }
        }));
    }

    pending.retain(|entry| {
        !summaries.iter().any(|request| {
            same_release(
                &request.label,
                request.artist.as_deref(),
                &entry.label,
                entry.artist.as_deref(),
            )
        })
    });

    let mut deduped: Vec<PendingRequestSummary> = Vec::with_capacity(pending.len());
    for entry in pending {
        match deduped.iter_mut().find(|existing| {
            same_release(
                &existing.label,
                existing.artist.as_deref(),
                &entry.label,
                entry.artist.as_deref(),
            )
        }) {
            Some(existing) => {
                if entry.state == "searching" {
                    *existing = entry;
                }
            }
            None => deduped.push(entry),
        }
    }

    Ok(DownloadRequestsOverview {
        requests: summaries,
        pending: deduped,
    })
}

#[cfg(feature = "server")]
fn same_release(
    left_label: &str,
    left_artist: Option<&str>,
    right_label: &str,
    right_artist: Option<&str>,
) -> bool {
    shared::library::album_activity_key(left_artist.unwrap_or_default(), left_label)
        == shared::library::album_activity_key(right_artist.unwrap_or_default(), right_label)
}

#[post("/api/downloads/requests/stop", auth: AuthSession)]
pub async fn stop_download_request(action: RequestAction) -> Result<(), ServerFnError> {
    let request = owned_request(&action.request_id, &auth.0.sub).await?;
    DownloadRequestRow::set_state(&request.id, "stopped")
        .await
        .map_err(server_error)?;
    cancel_request_token(&request.id).await;

    let tracks = DownloadRequestTrackRow::list_for_request(&request.id)
        .await
        .map_err(server_error)?;
    cancel_tracks(&request, &tracks, false)
        .await
        .map_err(server_error)?;
    DownloadRequestTrackRow::mark_stopped(&request.id)
        .await
        .map_err(server_error)?;

    let (tx, _) = get_or_create_user_channel(&auth.0.username).await;
    let entries: Vec<DownloadProgress> = tracks
        .iter()
        .filter(|t| !t.state_is_terminal())
        .map(|t| progress_from_track(&request, t, DownloadState::Stopped, None))
        .collect();
    if !entries.is_empty() {
        broadcast_progress(&tx, entries).await;
    }
    info!("User {} stopped request {}", auth.0.username, request.id);
    Ok(())
}

#[post("/api/downloads/requests/resume", auth: AuthSession)]
pub async fn resume_download_request(action: RequestAction) -> Result<(), ServerFnError> {
    let request = owned_request(&action.request_id, &auth.0.sub).await?;
    let tracks = DownloadRequestTrackRow::list_for_request(&request.id)
        .await
        .map_err(server_error)?;
    let pending: Vec<_> = tracks
        .into_iter()
        .filter(|t| {
            matches!(
                t.state.as_str(),
                "stopped" | "cancelled" | "queued" | "in_progress" | "completed" | "importing"
            )
        })
        .collect();
    let (tx, _) = get_or_create_user_channel(&auth.0.username).await;
    requeue(&auth.0.username, &request, pending, &tx)
        .await
        .map_err(server_error)?;
    Ok(())
}

#[post("/api/downloads/requests/retry", auth: AuthSession)]
pub async fn retry_download_request(action: RequestAction) -> Result<(), ServerFnError> {
    let request = owned_request(&action.request_id, &auth.0.sub).await?;
    let tracks = DownloadRequestTrackRow::list_for_request(&request.id)
        .await
        .map_err(server_error)?;
    let failed: Vec<_> = tracks.into_iter().filter(|t| t.state == "failed").collect();
    let (tx, _) = get_or_create_user_channel(&auth.0.username).await;
    requeue(&auth.0.username, &request, failed, &tx)
        .await
        .map_err(server_error)?;
    Ok(())
}

#[post("/api/downloads/requests/remove", auth: AuthSession)]
pub async fn remove_download_request(action: RequestAction) -> Result<(), ServerFnError> {
    let request = owned_request(&action.request_id, &auth.0.sub).await?;
    cancel_request_token(&request.id).await;
    let tracks = DownloadRequestTrackRow::list_for_request(&request.id)
        .await
        .map_err(server_error)?;
    cancel_tracks(&request, &tracks, true)
        .await
        .map_err(server_error)?;

    let download_base = crate::models::app_config::AppConfig::download_path().await;
    let slskd_download_base = crate::models::app_config::AppConfig::slskd_download_path().await;
    for track in &tracks {
        if let Some(path) = resolve_download_path_mapped(
            &track.item_key,
            &download_base,
            slskd_download_base.as_deref(),
        ) {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                warn!("Could not remove downloaded file {}: {}", path, e);
            }
        }
    }

    let removed_items: Vec<String> = tracks.iter().map(|t| t.item_key.clone()).collect();
    crate::import_queue::forget_retries(&auth.0.username, &removed_items).await;
    DownloadRequestRow::delete(&request.id)
        .await
        .map_err(server_error)?;
    forget_request_token(&request.id).await;

    let (tx, _) = get_or_create_user_channel(&auth.0.username).await;
    let _ = tx.send(DownloadEvent::RequestRemoved {
        request_id: request.id.clone(),
        items: removed_items,
    });
    info!("User {} removed request {}", auth.0.username, request.id);
    Ok(())
}

#[post("/api/downloads/requests/pending/cancel", auth: AuthSession)]
pub async fn cancel_pending_request(action: PendingAction) -> Result<(), ServerFnError> {
    if crate::globals::cancel_pending_search(&auth.0.username, &action.id) {
        info!(
            "User {} cancelled the running search for {}",
            auth.0.username, action.id
        );
        return Ok(());
    }
    LibraryRequestRow::delete_for_user(&action.id, &auth.0.sub)
        .await
        .map_err(server_error)?;
    info!(
        "User {} cancelled the queued request {}",
        auth.0.username, action.id
    );
    Ok(())
}

#[post("/api/downloads/requests/dismiss-failed", auth: AuthSession)]
pub async fn dismiss_failed_download_requests() -> Result<Vec<String>, ServerFnError> {
    let removed = DownloadRequestRow::delete_failed(&auth.0.sub)
        .await
        .map_err(server_error)?;
    if !removed.is_empty() {
        let (tx, _) = get_or_create_user_channel(&auth.0.username).await;
        for request_id in &removed {
            forget_request_token(request_id).await;
            let _ = tx.send(DownloadEvent::RequestRemoved {
                request_id: request_id.clone(),
                items: Vec::new(),
            });
        }
    }
    Ok(removed)
}

#[post("/api/downloads/requests/dismiss-finished", auth: AuthSession)]
pub async fn dismiss_finished_download_requests() -> Result<(), ServerFnError> {
    let removed = DownloadRequestRow::delete_finished(&auth.0.sub)
        .await
        .map_err(server_error)?;
    if removed.is_empty() {
        return Ok(());
    }
    let (tx, _) = get_or_create_user_channel(&auth.0.username).await;
    for request_id in removed {
        forget_request_token(&request_id).await;
        let _ = tx.send(DownloadEvent::RequestRemoved {
            request_id,
            items: Vec::new(),
        });
    }
    Ok(())
}

#[cfg(feature = "server")]
impl DownloadRequestTrackRow {
    pub fn state_is_terminal(&self) -> bool {
        state_is_terminal_key(&self.state)
    }
}
