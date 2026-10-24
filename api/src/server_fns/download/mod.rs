use dioxus::fullstack::{WebSocketOptions, Websocket};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(feature = "server")]
use shared::download::DownloadProgress;
use shared::download::{DownloadEvent, DownloadRequestContext, DownloadableItem, QueuedDownload};

#[cfg(feature = "server")]
use dioxus::logger::tracing::{info, warn};
#[cfg(feature = "server")]
use tokio::sync::broadcast;

#[cfg(feature = "server")]
use crate::{server_fns::server_error, AuthSession};

#[cfg(feature = "server")]
use crate::globals::{cleanup_stale_channels, get_or_create_user_channel, USER_CHANNELS};
#[cfg(feature = "server")]
use crate::services::download_backend;

// Local modules
pub mod auto_download;
pub use auto_download::{auto_download, AutoDownloadRequest, AutoDownloadResult};

pub mod requests;
pub use requests::*;

#[cfg(feature = "server")]
pub mod failover;
#[cfg(feature = "server")]
pub mod import;
#[cfg(feature = "server")]
pub mod monitor;
#[cfg(feature = "server")]
pub mod process;
#[cfg(feature = "server")]
pub mod utils;

// Shared with the tests of sibling modules outside `download`, so it cannot
// stay private to this one.
#[cfg(all(test, feature = "server"))]
pub(crate) mod test_support;

#[cfg(feature = "server")]
async fn do_download(
    items: Vec<DownloadableItem>,
    backend_id: Option<&str>,
) -> Result<Vec<QueuedDownload>, ServerFnError> {
    let backend = download_backend(backend_id)
        .await
        .map_err(|e| server_error(format!("download backend not available: {}", e)))?;

    backend.download(items).await.map_err(server_error)
}

/// WebSocket endpoint for real-time download updates.
/// Uses WebSocket instead of HTTP streaming for more reliable delivery.
#[get("/api/downloads/updates", auth: AuthSession)]
pub async fn download_updates_ws(
    options: WebSocketOptions,
) -> Result<Websocket<(), DownloadEvent>, ServerFnError> {
    let username = auth.0.username;

    let rx = {
        let map = USER_CHANNELS.read().await;
        if let Some(channel) = map.get(&username) {
            channel.sender.subscribe()
        } else {
            drop(map);
            let mut map = USER_CHANNELS.write().await;
            let channel = map
                .entry(username.clone())
                .or_insert_with(crate::globals::UserChannel::new);
            channel.sender.subscribe()
        }
    };

    Ok(options.on_upgrade(move |mut socket| async move {
        let mut rx = rx;
        info!("WebSocket connected for user: {}", username);

        loop {
            // handle both broadcast messages and potential socket closure
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(downloads) => {
                            if socket.send(downloads).await.is_err() {
                                info!("WebSocket closed (client disconnected)");
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(
                                "Download updates lagged, skipped {} messages. Consider reducing download batch size.",
                                skipped
                            );
                            // lagged is recoverable - continue receiving
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("Broadcast channel closed");
                            break;
                        }
                    }
                }
                result = socket.recv() => {
                    if result.is_err() {
                        info!("WebSocket client disconnected");
                        break;
                    }
                }
            }
        }

        info!("WebSocket disconnected for user: {}", username);

        // Trigger cleanup of stale channels periodically
        cleanup_stale_channels().await;
    }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelDownloadRequest {
    pub id: String,
    pub source: String,
    pub item: String,
    pub backend: Option<String>,
}

#[post("/api/downloads/cancel", auth: AuthSession)]
pub async fn cancel_download(req: CancelDownloadRequest) -> Result<(), ServerFnError> {
    let username = auth.0.username;
    let user_id = auth.0.sub.clone();

    let backend = download_backend(req.backend.as_deref())
        .await
        .map_err(|e| server_error(format!("download backend not available: {}", e)))?;

    let transfer_id = if uuid::Uuid::parse_str(&req.id).is_ok() {
        req.id.clone()
    } else {
        backend
            .get_downloads_for(std::slice::from_ref(&req.source))
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|dl| dl.source == req.source && dl.item == req.item)
            .map(|dl| dl.id)
            .unwrap_or_else(|| req.id.clone())
    };

    backend
        .cancel_download(&req.source, &transfer_id, false)
        .await
        .map_err(server_error)?;

    info!(
        "User {} cancelled download {} from {}",
        username, transfer_id, req.source
    );

    let mut cancelled = DownloadProgress {
        id: req.id,
        source: req.source,
        item: req.item.clone(),
        size: 0,
        transferred: 0,
        state: shared::download::DownloadState::Cancelled,
        percent: 0.0,
        speed: 0.0,
        error: None,
        backend: req.backend,
        batch_id: None,
        batch_label: None,
    };

    if let Ok(Some(track)) =
        crate::models::download_request::DownloadRequestTrackRow::find_by_item(&user_id, &req.item)
            .await
    {
        let _ = crate::models::download_request::DownloadRequestTrackRow::set_state(
            &track.request_id,
            &track.item_key,
            &cancelled.state,
            None,
        )
        .await;
        if let Ok(Some(request)) =
            crate::models::download_request::DownloadRequestRow::get_for_user(
                &track.request_id,
                &user_id,
            )
            .await
        {
            cancelled.batch_id = Some(request.id);
            cancelled.batch_label = Some(request.label);
        }
    }

    // Send cancelled state to UI via broadcast channel
    let (tx, _) = get_or_create_user_channel(&username).await;
    requests::broadcast_progress(&tx, vec![cancelled]).await;

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub items: Vec<DownloadableItem>,
    pub target_folder: String,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub request: DownloadRequestContext,
}

#[post("/api/downloads/queue", auth: AuthSession)]
pub async fn download(req: DownloadRequest) -> Result<Vec<QueuedDownload>, ServerFnError> {
    let username = auth.0.username;
    let user_id = auth.0.sub;

    let target_path_buf = std::path::Path::new(&req.target_folder).to_path_buf();
    if let Err(e) = tokio::fs::create_dir_all(&target_path_buf).await {
        return Err(server_error(format!(
            "Failed to create target directory: {}",
            e
        )));
    }

    let context = if req.request.key.is_empty() {
        let first = req.items.first();
        DownloadRequestContext {
            key: format!(
                "album:{}",
                first.map(|item| item.album.as_str()).unwrap_or_default()
            ),
            label: first
                .map(|item| item.album.clone())
                .unwrap_or_else(|| "Unknown release".to_string()),
            artist: first.map(|item| item.artist.clone()),
        }
    } else {
        req.request
    };

    let request = requests::open_request(
        &user_id,
        &context,
        &req.target_folder,
        req.backend.as_deref(),
    )
    .await
    .map_err(server_error)?;

    let new_items = requests::insert_new_items(&request, req.items)
        .await
        .map_err(server_error)?;
    if new_items.is_empty() {
        info!("Request {} already holds every selected track", request.id);
        return Ok(Vec::new());
    }

    let res = match do_download(new_items.clone(), req.backend.as_deref()).await {
        Ok(res) => res,
        Err(e) => {
            for item in &new_items {
                let _ = crate::models::download_request::DownloadRequestTrackRow::mark_failed(
                    &request.id,
                    &item.id,
                    &e.to_string(),
                )
                .await;
            }
            return Err(e);
        }
    };

    let (tx, _) = get_or_create_user_channel(&username).await;
    let accepted = requests::record_queued(&request, &new_items, &res, &tx).await;
    requests::start_monitor(&username, &user_id, &request, accepted).await;

    info!(
        "Started {} downloads for request {} ({})",
        new_items.len(),
        request.id,
        request.label
    );

    Ok(res)
}
