use dioxus::prelude::*;
use shared::system::{AvailableBackends, SystemHealth};

#[cfg(feature = "server")]
use shared::system::BackendInfo;

#[cfg(feature = "server")]
use crate::services::{
    available_download_backends, available_importers, available_metadata_providers,
    download_backend, evict_media_server, media_server_for_user, music_importer,
};
#[cfg(feature = "server")]
use crate::AuthSession;
#[cfg(feature = "server")]
use dioxus::logger::tracing::debug;

#[get("/api/system/health", auth: AuthSession)]
pub async fn get_system_health() -> Result<SystemHealth, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let downloader_online = match download_backend(None).await {
            Ok(backend) => backend.health_check().await,
            Err(_) => false,
        };

        let beets_ready = match music_importer(None).await {
            Ok(importer) => importer.health_check().await,
            Err(_) => false,
        };

        let media_server_online = match media_server_for_user(&auth.0.sub).await {
            Ok(server) => match server.ping().await {
                Ok(()) => true,
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("Circuit breaker open") {
                        debug!(
                            "Evicted stale media-server client for user {} after circuit-breaker-open ping",
                            auth.0.sub
                        );
                        evict_media_server(&auth.0.sub).await;
                    }
                    false
                }
            },
            Err(_) => false,
        };

        Ok(SystemHealth {
            downloader_online,
            beets_ready,
            media_server_online,
        })
    }
    #[cfg(not(feature = "server"))]
    Ok(SystemHealth::default())
}

#[get("/api/system/backends", _: AuthSession)]
pub async fn get_backends() -> Result<AvailableBackends, ServerFnError> {
    #[cfg(feature = "server")]
    {
        Ok(AvailableBackends {
            metadata: available_metadata_providers()
                .await
                .into_iter()
                .map(|(id, name)| BackendInfo { id, name })
                .collect(),
            download: available_download_backends()
                .into_iter()
                .map(|(id, name)| BackendInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                })
                .collect(),
            importer: available_importers()
                .into_iter()
                .map(|(id, name)| BackendInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                })
                .collect(),
        })
    }
    #[cfg(not(feature = "server"))]
    Ok(AvailableBackends::default())
}
