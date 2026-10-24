#[cfg(feature = "server")]
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

#[cfg(feature = "server")]
use soulbeet::{
    beets::BeetsImporter,
    musicbrainz::MusicBrainzProvider,
    navidrome::NavidromeClientBuilder,
    plex::PlexClientBuilder,
    slskd::{DownloadConfig, SoulseekClientBuilder},
    DownloadBackend, LastFmProvider, MediaServer, MetadataProvider, MusicImporter,
};
#[cfg(feature = "server")]
use tokio::sync::RwLock;

#[cfg(feature = "server")]
use shared::media_server::{AuthServers, MediaServerKind};

#[cfg(feature = "server")]
use crate::models::app_config::{keys, AppConfig};

pub mod providers {
    pub const MUSICBRAINZ: &str = "musicbrainz";
    pub const BRAINZMASH: &str = "brainzmash";
    pub const LASTFM: &str = "lastfm";
}

pub mod downloaders {
    pub const SLSKD: &str = "slskd";
}

pub mod importers {
    pub const BEETS: &str = "beets";
}

#[cfg(feature = "server")]
static METADATA_PROVIDERS: LazyLock<RwLock<HashMap<String, Arc<dyn MetadataProvider>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(feature = "server")]
static DOWNLOAD_BACKENDS: LazyLock<RwLock<HashMap<String, Arc<dyn DownloadBackend>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(feature = "server")]
static MUSIC_IMPORTERS: LazyLock<RwLock<HashMap<String, Arc<dyn MusicImporter>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(feature = "server")]
static MEDIA_SERVERS: LazyLock<RwLock<HashMap<String, Arc<dyn MediaServer>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[cfg(feature = "server")]
pub fn available_metadata_providers() -> Vec<(&'static str, &'static str)> {
    vec![
        (providers::MUSICBRAINZ, "MusicBrainz"),
        (providers::BRAINZMASH, "BrainzMash"),
        (providers::LASTFM, "Last.fm"),
    ]
}

#[cfg(feature = "server")]
pub fn available_download_backends() -> Vec<(&'static str, &'static str)> {
    vec![(downloaders::SLSKD, "Soulseek")]
}

#[cfg(feature = "server")]
pub fn available_importers() -> Vec<(&'static str, &'static str)> {
    vec![(importers::BEETS, "Beets")]
}

#[cfg(feature = "server")]
async fn init_metadata_provider(
    id: &str,
    lastfm_api_key: Option<&str>,
) -> Result<Arc<dyn MetadataProvider>, String> {
    match id {
        providers::BRAINZMASH => Ok(Arc::new(MusicBrainzProvider::brainzmash())),
        providers::LASTFM => {
            let api_key = lastfm_api_key
                .filter(|k| !k.is_empty())
                .ok_or("Last.fm API key not configured")?;
            Ok(Arc::new(LastFmProvider::new(api_key.to_string())))
        }
        _ => Ok(Arc::new(MusicBrainzProvider::new())),
    }
}

#[cfg(feature = "server")]
async fn init_download_backend(id: &str) -> Result<Arc<dyn DownloadBackend>, String> {
    match id {
        downloaders::SLSKD => {
            let url = AppConfig::get(keys::SLSKD_URL)
                .await?
                .ok_or("slskd URL not configured")?;
            let api_key = AppConfig::get(keys::SLSKD_API_KEY)
                .await?
                .ok_or("slskd API key not configured")?;

            if url.is_empty() || api_key.is_empty() {
                return Err("slskd not configured".to_string());
            }

            let client = SoulseekClientBuilder::new()
                .base_url(&url)
                .api_key(&api_key)
                .download_config(DownloadConfig {
                    batch_size: 3,
                    batch_delay_ms: 3000,
                    max_retries: 3,
                    retry_base_delay_ms: 1000,
                })
                .build()
                .map_err(|e| e.to_string())?;

            Ok(Arc::new(client))
        }
        _ => Err(format!("Unknown download backend: {}", id)),
    }
}

#[cfg(feature = "server")]
pub async fn metadata_provider(
    id: Option<&str>,
    lastfm_api_key: Option<&str>,
) -> Result<Arc<dyn MetadataProvider>, String> {
    let requested = id.unwrap_or(providers::MUSICBRAINZ);

    // Don't cache Last.fm providers since keys are per-user
    if requested != providers::LASTFM {
        if let Some(provider) = METADATA_PROVIDERS.read().await.get(requested) {
            return Ok(provider.clone());
        }
    }

    let (key, provider) = match init_metadata_provider(requested, lastfm_api_key).await {
        Ok(p) => (requested.to_string(), p),
        Err(_) if requested != providers::MUSICBRAINZ => (
            providers::MUSICBRAINZ.to_string(),
            init_metadata_provider(providers::MUSICBRAINZ, None).await?,
        ),
        Err(e) => return Err(e),
    };

    // Only cache non-Last.fm providers
    if key != providers::LASTFM {
        METADATA_PROVIDERS
            .write()
            .await
            .insert(key, provider.clone());
    }
    Ok(provider)
}

#[cfg(feature = "server")]
pub async fn download_backend(id: Option<&str>) -> Result<Arc<dyn DownloadBackend>, String> {
    let requested = id.unwrap_or(downloaders::SLSKD);

    if let Some(backend) = DOWNLOAD_BACKENDS.read().await.get(requested) {
        return Ok(backend.clone());
    }

    let backend = init_download_backend(requested).await?;
    DOWNLOAD_BACKENDS
        .write()
        .await
        .insert(requested.to_string(), backend.clone());
    Ok(backend)
}

#[cfg(feature = "server")]
fn init_importer(id: &str) -> Result<Arc<dyn MusicImporter>, String> {
    match id {
        importers::BEETS => Ok(Arc::new(BeetsImporter::from_env())),
        _ => Err(format!("Unknown importer: {}", id)),
    }
}

#[cfg(feature = "server")]
pub async fn music_importer(id: Option<&str>) -> Result<Arc<dyn MusicImporter>, String> {
    let requested = id.unwrap_or(importers::BEETS);

    if let Some(importer) = MUSIC_IMPORTERS.read().await.get(requested) {
        return Ok(importer.clone());
    }

    let importer = init_importer(requested)?;
    MUSIC_IMPORTERS
        .write()
        .await
        .insert(requested.to_string(), importer.clone());
    Ok(importer)
}

#[cfg(feature = "server")]
pub async fn media_server_for_user(user_id: &str) -> Result<Arc<dyn MediaServer>, String> {
    if let Some(server) = MEDIA_SERVERS.read().await.get(user_id) {
        return Ok(server.clone());
    }

    let user = crate::models::user::User::get_by_id(user_id).await?;
    let kind = MediaServerKind::from(user.media_server.as_str());
    let config = crate::models::media_server::MediaServerConfig::get(user_id, kind)
        .await?
        .ok_or("Media server not configured for user")?;

    let encrypted_token = config
        .token
        .as_deref()
        .ok_or("Media server not configured for user")?;
    let secret = crate::crypto::decrypt(encrypted_token)?;

    let server: Arc<dyn MediaServer> = match kind {
        MediaServerKind::Navidrome => {
            let url = config
                .url
                .as_deref()
                .ok_or("Navidrome URL is not configured")?;
            let username = config.account_name.as_deref().unwrap_or(&user.username);
            Arc::new(
                NavidromeClientBuilder::new()
                    .base_url(url)
                    .username(username)
                    .password(&secret)
                    .build()
                    .map_err(|e| e.to_string())?,
            )
        }
        MediaServerKind::Plex => {
            let url = config
                .url
                .as_deref()
                .ok_or("Plex server URL is not configured")?;
            let client_id = plex_client_id().await?;
            Arc::new(
                PlexClientBuilder::new()
                    .base_url(url)
                    .token(&secret)
                    .client_id(&client_id)
                    .build()
                    .map_err(|e| e.to_string())?,
            )
        }
    };

    MEDIA_SERVERS
        .write()
        .await
        .insert(user_id.to_string(), server.clone());
    Ok(server)
}

#[cfg(feature = "server")]
pub async fn evict_media_server(user_id: &str) {
    MEDIA_SERVERS.write().await.remove(user_id);
}

#[cfg(feature = "server")]
pub async fn available_media_servers() -> AuthServers {
    use crate::models::media_server::MediaServerConfig;

    AuthServers {
        navidrome: MediaServerConfig::any_configured(MediaServerKind::Navidrome)
            .await
            .unwrap_or(false),
        plex: MediaServerConfig::any_configured(MediaServerKind::Plex)
            .await
            .unwrap_or(false),
    }
}

#[cfg(feature = "server")]
pub async fn plex_client_id() -> Result<String, String> {
    if let Some(id) = AppConfig::get(keys::PLEX_CLIENT_ID).await? {
        if !id.is_empty() {
            return Ok(id);
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    AppConfig::set(keys::PLEX_CLIENT_ID, &id).await?;
    Ok(id)
}

#[cfg(feature = "server")]
pub async fn reload_providers() {
    METADATA_PROVIDERS.write().await.clear();
    DOWNLOAD_BACKENDS.write().await.clear();
    MUSIC_IMPORTERS.write().await.clear();
    MEDIA_SERVERS.write().await.clear();
}

#[cfg(feature = "server")]
pub async fn is_slskd_configured() -> bool {
    let url = AppConfig::get(keys::SLSKD_URL).await.ok().flatten();
    let api_key = AppConfig::get(keys::SLSKD_API_KEY).await.ok().flatten();
    matches!((url, api_key), (Some(u), Some(k)) if !u.is_empty() && !k.is_empty())
}
