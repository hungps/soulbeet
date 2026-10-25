use reqwest::{Client, Method, Response};
use tracing::{info, warn};
use url::Url;

use crate::error::{Result, SoulseekError};
use crate::http::{build_client, resilient_send, resolve_docker_url, CircuitBreaker};
use crate::traits::{MediaServer, PlaylistSpec};
use shared::media_server::{MediaAlbum, MediaServerKind, MediaTrack};

use super::models::*;

const PAGE_SIZE: u32 = 1000;
const TRACK_TYPE: &str = "10";
const ALBUM_TYPE: &str = "9";
const MUSIC_SECTION_TYPE: &str = "artist";

pub struct PlexClient {
    base_url: Url,
    token: String,
    client_id: String,
    music_section: Option<String>,
    client: Client,
    circuit_breaker: CircuitBreaker,
    machine_identifier: tokio::sync::Mutex<Option<String>>,
}

#[derive(Default)]
pub struct PlexClientBuilder {
    base_url: Option<String>,
    token: Option<String>,
    client_id: Option<String>,
    music_section: Option<String>,
}

impl PlexClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn base_url(mut self, url: &str) -> Self {
        self.base_url = Some(url.to_string());
        self
    }

    pub fn token(mut self, token: &str) -> Self {
        self.token = Some(token.to_string());
        self
    }

    pub fn client_id(mut self, client_id: &str) -> Self {
        self.client_id = Some(client_id.to_string());
        self
    }

    pub fn music_section(mut self, section: Option<&str>) -> Self {
        self.music_section = section.map(|s| s.to_string());
        self
    }

    pub fn build(self) -> Result<PlexClient> {
        let base_url = self.base_url.ok_or(SoulseekError::NotConfigured)?;
        let resolved = resolve_docker_url(&base_url);
        let mut normalized = resolved.trim_end_matches('/').to_string();
        normalized.push('/');
        let base_url = Url::parse(&normalized)?;

        let token = self
            .token
            .filter(|t| !t.is_empty())
            .ok_or_else(|| SoulseekError::Api {
                status: 0,
                message: "Plex token not configured".to_string(),
            })?;
        let client_id = self.client_id.ok_or_else(|| SoulseekError::Api {
            status: 0,
            message: "Plex client identifier not configured".to_string(),
        })?;

        Ok(PlexClient {
            base_url,
            token,
            client_id,
            music_section: self.music_section,
            client: build_client("Soulbeet"),
            circuit_breaker: CircuitBreaker::default(),
            machine_identifier: tokio::sync::Mutex::new(None),
        })
    }
}

impl PlexClient {
    fn url(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|e| SoulseekError::Api {
                status: 0,
                message: format!("URL error: {}", e),
            })
    }

    async fn send(&self, method: Method, path: &str, query: &[(&str, String)]) -> Result<Response> {
        self.send_with_headers(method, path, query, &[], true).await
    }

    async fn send_mutation(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Response> {
        self.send_with_headers(method, path, query, &[], false)
            .await
    }

    async fn send_with_headers(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        headers: &[(&str, String)],
        retry: bool,
    ) -> Result<Response> {
        if self.circuit_breaker.is_open().await {
            return Err(SoulseekError::Api {
                status: 503,
                message: "Circuit breaker open - Plex appears unavailable".to_string(),
            });
        }

        let url = self.url(path)?;
        let client = self.client.clone();
        let token = self.token.clone();
        let client_id = self.client_id.clone();
        let query: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        let headers: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        let method_name = format!("{} {}", method, path);

        let build_request = || {
            let mut req = client
                .request(method.clone(), url.clone())
                .header("X-Plex-Token", &token)
                .header("X-Plex-Client-Identifier", &client_id)
                .header("Accept", "application/json")
                .query(&query);
            for (name, value) in &headers {
                req = req.header(name, value);
            }
            req
        };

        let resp = if retry {
            resilient_send(build_request, &format!("Plex {}", method_name)).await
        } else {
            build_request()
                .send()
                .await
                .map_err(|e| SoulseekError::Api {
                    status: 0,
                    message: format!("Plex {}: {}", method_name, e),
                })
        };

        match resp {
            Ok(resp) => {
                self.circuit_breaker.record_success().await;
                Ok(resp)
            }
            Err(e) => {
                self.circuit_breaker.record_failure().await;
                Err(e)
            }
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let resp = match self.send(Method::GET, path, query).await {
            Ok(r) => r,
            Err(e) => return Err(map_auth_error(e)),
        };
        let status = resp.status();
        let body = resp.text().await.map_err(|e| SoulseekError::Api {
            status: 500,
            message: format!("Failed to read Plex response: {}", e),
        })?;
        if !status.is_success() {
            return Err(SoulseekError::Api {
                status: status.as_u16(),
                message: format!("Plex {} failed: {}", path, status),
            });
        }
        serde_json::from_str(&body).map_err(|e| SoulseekError::Api {
            status: 500,
            message: format!("Failed to parse Plex response: {}", e),
        })
    }

    pub async fn ping(&self) -> Result<()> {
        // `identity` is unauthenticated, so it cannot validate the token.
        let _: PlexEnvelope<PlexSectionsContainer> = self.get_json("library/sections", &[]).await?;
        Ok(())
    }

    pub async fn machine_identifier(&self) -> Result<String> {
        if let Some(id) = self.machine_identifier.lock().await.clone() {
            return Ok(id);
        }
        let envelope: PlexEnvelope<PlexIdentityContainer> = self.get_json("identity", &[]).await?;
        let id = envelope.media_container.machine_identifier;
        if id.is_empty() {
            return Err(SoulseekError::Api {
                status: 500,
                message: "Plex did not return a machine identifier".to_string(),
            });
        }
        *self.machine_identifier.lock().await = Some(id.clone());
        Ok(id)
    }

    pub async fn music_section_keys(&self) -> Result<Vec<String>> {
        if let Some(section) = &self.music_section {
            return Ok(vec![section.clone()]);
        }
        let envelope: PlexEnvelope<PlexSectionsContainer> =
            self.get_json("library/sections", &[]).await?;
        let sections: Vec<String> = envelope
            .media_container
            .directories
            .into_iter()
            .filter(|d| d.r#type == MUSIC_SECTION_TYPE)
            .map(|d| d.key)
            .collect();
        if sections.is_empty() {
            return Err(SoulseekError::Api {
                status: 404,
                message: "Plex has no music library configured".to_string(),
            });
        }
        Ok(sections)
    }

    async fn fetch_all_metadata(&self, item_type: &str) -> Result<Vec<PlexMetadata>> {
        let mut all = Vec::new();
        for section in self.music_section_keys().await? {
            let mut section_items = self.fetch_section_metadata(&section, item_type).await?;
            all.append(&mut section_items);
        }
        Ok(all)
    }

    async fn fetch_section_metadata(
        &self,
        section: &str,
        item_type: &str,
    ) -> Result<Vec<PlexMetadata>> {
        let path = format!("library/sections/{}/all", section);
        let mut all = Vec::new();
        let mut start = 0u32;

        loop {
            let start_str = start.to_string();
            let size_str = PAGE_SIZE.to_string();
            let resp = match self
                .send_with_headers(
                    Method::GET,
                    &path,
                    &[
                        ("type", item_type.to_string()),
                        ("includeUserRating", "1".to_string()),
                    ],
                    &[
                        ("X-Plex-Container-Start", start_str),
                        ("X-Plex-Container-Size", size_str),
                    ],
                    true,
                )
                .await
            {
                Ok(r) => r,
                Err(e) => return Err(map_auth_error(e)),
            };

            let status = resp.status();
            if !status.is_success() {
                return Err(map_auth_error(SoulseekError::Api {
                    status: status.as_u16(),
                    message: format!("Plex library request failed: {}", status),
                }));
            }

            let page: PlexEnvelope<PlexMetadataContainer> =
                resp.json().await.map_err(|e| SoulseekError::Api {
                    status: 500,
                    message: format!("Failed to parse Plex library response: {}", e),
                })?;

            let container = page.media_container;
            let count = container.metadata.len() as u32;
            let total = container.total_size.unwrap_or(container.size);
            let page_start = container.offset.unwrap_or(start);
            all.extend(container.metadata);

            let next = page_start.saturating_add(count);
            if count == 0 || next <= start {
                break;
            }
            start = next;
            if total > 0 && start >= total {
                break;
            }
        }

        Ok(all)
    }

    pub async fn get_all_tracks(&self) -> Result<Vec<MediaTrack>> {
        let metadata = self.fetch_all_metadata(TRACK_TYPE).await?;
        info!("Fetched {} tracks from Plex", metadata.len());
        Ok(metadata.into_iter().map(map_track).collect())
    }

    pub async fn get_all_albums(&self) -> Result<Vec<MediaAlbum>> {
        let metadata = self.fetch_all_metadata(ALBUM_TYPE).await?;
        Ok(metadata
            .into_iter()
            .map(|album| {
                let artist = album.artist();
                let genre = album.genre();
                MediaAlbum {
                    id: album.rating_key,
                    name: album.title,
                    artist,
                    song_count: album.leaf_count,
                    genre,
                }
            })
            .collect())
    }

    async fn find_playlist_by_title(&self, title: &str) -> Result<Option<String>> {
        let envelope: PlexEnvelope<PlexPlaylistsContainer> =
            self.get_json("playlists", &[]).await?;
        Ok(select_playlist_id(envelope.media_container, title))
    }

    fn item_uri(&self, machine_id: &str, rating_key: &str) -> String {
        format!(
            "server://{}/com.plexapp.plugins.library/library/metadata/{}",
            machine_id, rating_key
        )
    }

    pub async fn create_playlist(
        &self,
        name: &str,
        first_rating_key: &str,
    ) -> Result<Option<String>> {
        let machine_id = self.machine_identifier().await?;
        let first_uri = self.item_uri(&machine_id, first_rating_key);

        let resp = self
            .send_mutation(
                Method::POST,
                "playlists",
                &[
                    ("type", "audio".to_string()),
                    ("title", name.to_string()),
                    ("smart", "0".to_string()),
                    ("uri", first_uri),
                ],
            )
            .await
            .map_err(map_auth_error)?;

        let status = resp.status();
        if !status.is_success() {
            return Err(map_auth_error(SoulseekError::Api {
                status: status.as_u16(),
                message: format!("Plex create playlist failed: {}", status),
            }));
        }

        let body = resp.text().await.unwrap_or_default();
        let playlist_id = serde_json::from_str::<PlexEnvelope<PlexPlaylistsContainer>>(&body)
            .ok()
            .and_then(|body| {
                body.media_container
                    .metadata
                    .into_iter()
                    .chain(body.media_container.playlists)
                    .find(|playlist| !playlist.rating_key.is_empty())
                    .map(|playlist| playlist.rating_key)
            })
            .or(self.find_playlist_by_title(name).await?);

        playlist_id.map(Some).ok_or_else(|| SoulseekError::Api {
            status: 500,
            message: format!("Plex did not return an id for playlist '{}'", name),
        })
    }

    async fn clear_playlist(&self, playlist_id: &str) -> Result<()> {
        let resp = self
            .send_mutation(
                Method::DELETE,
                &format!("playlists/{}/items", playlist_id),
                &[],
            )
            .await
            .map_err(map_auth_error)?;
        let status = resp.status();
        if status.is_success() || status.as_u16() == 404 {
            Ok(())
        } else {
            Err(SoulseekError::Api {
                status: status.as_u16(),
                message: format!("Plex clear playlist failed: {}", status),
            })
        }
    }

    async fn add_to_playlist(
        &self,
        machine_id: &str,
        playlist_id: &str,
        rating_key: &str,
    ) -> Result<()> {
        let uri = self.item_uri(machine_id, rating_key);
        let resp = self
            .send_mutation(
                Method::PUT,
                &format!("playlists/{}/items", playlist_id),
                &[("uri", uri)],
            )
            .await
            .map_err(map_auth_error)?;
        if !resp.status().is_success() {
            warn!(
                "Plex refused to add track {} to playlist {}: {}",
                rating_key,
                playlist_id,
                resp.status()
            );
        }
        Ok(())
    }

    pub async fn upsert_playlist(
        &self,
        name: &str,
        rating_keys: &[String],
    ) -> Result<Option<String>> {
        let Some(first_key) = rating_keys.first() else {
            return Ok(None);
        };
        let machine_id = self.machine_identifier().await?;

        let (playlist_id, first_new) = match self.find_playlist_by_title(name).await? {
            Some(id) => match self.clear_playlist(&id).await {
                Ok(()) => (Some(id), 0),
                Err(e) => {
                    warn!(
                        "Could not clear Plex playlist '{}' ({}); recreating it",
                        name, e
                    );
                    let _ = self.delete_playlist(&id).await;
                    (self.create_playlist(name, first_key).await?, 1)
                }
            },
            None => (self.create_playlist(name, first_key).await?, 1),
        };

        if let Some(id) = &playlist_id {
            for key in &rating_keys[first_new..] {
                self.add_to_playlist(&machine_id, id, key).await?;
            }
        }

        Ok(playlist_id)
    }

    pub async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let resp = self
            .send_mutation(Method::DELETE, &format!("playlists/{}", playlist_id), &[])
            .await
            .map_err(map_auth_error)?;
        if resp.status().is_success() || resp.status().as_u16() == 404 {
            Ok(())
        } else {
            Err(SoulseekError::Api {
                status: resp.status().as_u16(),
                message: format!("Plex delete playlist failed: {}", resp.status()),
            })
        }
    }

    pub async fn refresh_section(&self, path: Option<&str>) -> Result<()> {
        let query: Vec<(&str, String)> = path
            .map(|p| vec![("path", p.to_string())])
            .unwrap_or_default();
        for section in self.music_section_keys().await? {
            let resp = self
                .send(
                    Method::GET,
                    &format!("library/sections/{}/refresh", section),
                    &query,
                )
                .await
                .map_err(map_auth_error)?;
            if !resp.status().is_success() {
                return Err(SoulseekError::Api {
                    status: resp.status().as_u16(),
                    message: format!("Plex scan request failed: {}", resp.status()),
                });
            }
        }
        Ok(())
    }
}

fn map_auth_error(error: SoulseekError) -> SoulseekError {
    match error {
        SoulseekError::Api { status: 401, .. } => SoulseekError::Api {
            status: 401,
            message: "Plex rejected the token".to_string(),
        },
        other => other,
    }
}

fn map_track(track: PlexMetadata) -> MediaTrack {
    let artist = track.artist();
    let album = track.album();
    let genre = track.genre();
    let path = track.file_path();
    let user_rating = track.star_rating();
    MediaTrack {
        id: track.rating_key,
        title: track.title,
        artist,
        album,
        genre,
        duration_secs: track.duration.map(|ms| (ms / 1000) as u32),
        path,
        user_rating,
        average_rating: None,
        play_count: None,
    }
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Plex and Soulbeet often mount the same library at different roots, so a
/// folder is matched by the tail of its path. Two segments are used
/// (`Discovery/<Profile>`) because the last one alone is too weak a match.
fn folder_needle(folder: &str) -> String {
    let normalized = normalize_path(folder);
    let tail: Vec<&str> = normalized.rsplit('/').take(2).collect();
    format!("/{}/", tail.into_iter().rev().collect::<Vec<_>>().join("/"))
}

/// Rating keys of the library tracks that live under the discovery folder.
/// Resolving by folder instead of by saved file paths keeps the playlist
/// correct after beets renames the imported files.
pub(crate) fn resolve_folder_keys(folder: &str, library: &[MediaTrack]) -> Vec<String> {
    let needle = folder_needle(folder);
    library
        .iter()
        .filter(|track| {
            track
                .path
                .as_deref()
                .is_some_and(|path| normalize_path(path).contains(&needle))
        })
        .map(|track| track.id.clone())
        .collect()
}

fn select_playlist_id(container: PlexPlaylistsContainer, title: &str) -> Option<String> {
    container
        .metadata
        .into_iter()
        .chain(container.playlists)
        .filter(|playlist| {
            let audio = playlist.playlist_type.as_deref() == Some("audio")
                || (playlist.playlist_type.is_none()
                    && playlist.item_type.as_deref() == Some("playlist"));
            playlist.title == title && audio && playlist.smart != Some(true)
        })
        .max_by_key(|playlist| playlist.updated_at.unwrap_or(0))
        .map(|playlist| playlist.rating_key)
}

#[async_trait::async_trait]
impl MediaServer for PlexClient {
    fn kind(&self) -> MediaServerKind {
        MediaServerKind::Plex
    }

    async fn ping(&self) -> Result<()> {
        PlexClient::ping(self).await
    }

    async fn start_scan(&self) -> Result<()> {
        self.refresh_section(None).await
    }

    async fn start_scan_paths(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return self.refresh_section(None).await;
        }
        for path in paths {
            if let Err(e) = self.refresh_section(Some(path)).await {
                warn!(
                    "Plex could not refresh path '{}': {}; falling back to a full scan",
                    path, e
                );
                return self.refresh_section(None).await;
            }
        }
        Ok(())
    }

    async fn get_scan_status(&self) -> Result<bool> {
        // Best effort: Plex reports library refreshes as activities. Shared
        // users may not be allowed to list them, in which case the caller
        // proceeds to reconcile and the next automation cycle picks up files.
        let envelope: PlexEnvelope<PlexActivitiesContainer> =
            match self.get_json("activities", &[]).await {
                Ok(envelope) => envelope,
                Err(_) => return Ok(false),
            };

        Ok(envelope.media_container.activities.iter().any(|activity| {
            activity.kind.contains("library") && activity.progress.unwrap_or(0.0) < 100.0
        }))
    }

    async fn get_tracks(&self) -> Result<Vec<MediaTrack>> {
        self.get_all_tracks().await
    }

    async fn get_albums(&self) -> Result<Vec<MediaAlbum>> {
        self.get_all_albums().await
    }

    fn needs_library(&self) -> bool {
        true
    }

    async fn reconcile_playlist(&self, spec: &PlaylistSpec<'_>) -> Result<Option<String>> {
        let keys = resolve_folder_keys(spec.folder, spec.library);
        if keys.is_empty() {
            warn!(
                "No Plex tracks under '{}' for profile '{}', skipping playlist",
                spec.folder, spec.profile
            );
            return Ok(None);
        }

        let playlist_id = self.upsert_playlist(spec.name, &keys).await?;
        if let Some(id) = &playlist_id {
            info!(
                "Reconciled Plex playlist '{}' with {} tracks (id {})",
                spec.name,
                keys.len(),
                id
            );
        }
        Ok(playlist_id)
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        PlexClient::delete_playlist(self, playlist_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, path: &str) -> MediaTrack {
        MediaTrack {
            id: id.to_string(),
            title: "Track".to_string(),
            artist: None,
            album: None,
            genre: None,
            duration_secs: None,
            path: Some(path.to_string()),
            user_rating: None,
            average_rating: None,
            play_count: None,
        }
    }

    #[test]
    fn resolves_the_profile_folder_when_the_library_root_differs() {
        let library = vec![
            track("1", "/plex/music/Artist/Album/song.flac"),
            track(
                "2",
                "/medias/music-discovery/Discovery/Balanced/Artist/Album/song.flac",
            ),
            track(
                "3",
                "/medias/music-discovery/Discovery/Adventurous/Artist/Album/song.flac",
            ),
        ];
        let keys = resolve_folder_keys(
            "/Volumes/medias/music-discovery/Discovery/Balanced",
            &library,
        );
        assert_eq!(keys, vec!["2".to_string()]);
    }

    #[test]
    fn matches_the_folder_regardless_of_case_and_trailing_slashes() {
        let library = vec![track(
            "1",
            "/data/Music/Discovery/Fresh Picks/Artist/Album/song.flac",
        )];
        let keys = resolve_folder_keys("/volumes/music/Discovery/fresh picks/", &library);
        assert_eq!(keys, vec!["1".to_string()]);
    }

    #[test]
    fn keeps_tracks_of_other_profiles_out_of_the_playlist() {
        let library = vec![
            track("1", "/music/Discovery/Conservative/Artist/Album/song.flac"),
            track("2", "/music/Discovery/Conservative Deluxe/Artist/song.flac"),
        ];
        let keys = resolve_folder_keys("/music/Discovery/Conservative", &library);
        assert_eq!(keys, vec!["1".to_string()]);
    }

    #[test]
    fn skips_library_tracks_without_a_path() {
        let mut without_path = track("1", "/unused");
        without_path.path = None;
        let library = vec![without_path];
        assert!(resolve_folder_keys("/music/Discovery/Balanced", &library).is_empty());
    }

    #[test]
    fn finds_audio_playlists_in_metadata() {
        let json = r#"{"MediaContainer":{"size":4,"Metadata":[
            {"ratingKey":"folder","type":"directory","title":"Discovery"},
            {"ratingKey":"old","type":"playlist","title":"Discovery","playlistType":"audio","smart":0,"updatedAt":100},
            {"ratingKey":"new","type":"playlist","title":"Discovery","playlistType":"audio","smart":false,"updatedAt":200},
            {"ratingKey":"smart","type":"playlist","title":"Discovery","playlistType":"audio","smart":true,"updatedAt":300}
        ]}}"#;
        let envelope: PlexEnvelope<PlexPlaylistsContainer> = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_playlist_id(envelope.media_container, "Discovery"),
            Some("new".to_string())
        );
    }
}
