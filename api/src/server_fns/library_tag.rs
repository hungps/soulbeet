use dioxus::prelude::*;
#[cfg(feature = "server")]
use shared::download::{DownloadProgress, DownloadRequestContext, DownloadState, DownloadableItem};
#[cfg(feature = "server")]
use shared::library::{normalize_key, LibraryTagCandidate, LibraryTagFile, LibraryTagTrack};
use shared::library::{
    LibraryTagApplyRequest, LibraryTagApplyResult, LibraryTagPreview, LibraryTagPreviewRequest,
};
#[cfg(feature = "server")]
use shared::metadata::{SearchFilters, SearchResult, Track};

#[cfg(feature = "server")]
use crate::globals::get_or_create_user_channel;
#[cfg(feature = "server")]
use crate::models::download_request::{DownloadRequestRow, DownloadRequestTrackRow};
#[cfg(feature = "server")]
use crate::server_fns::download::requests::broadcast_progress;

#[cfg(feature = "server")]
use crate::models::folder::Folder;
#[cfg(feature = "server")]
use crate::models::library::{LibraryReleaseRow, LibraryTrackRow};
#[cfg(feature = "server")]
use crate::models::user_settings::UserSettings;
#[cfg(feature = "server")]
use crate::services::metadata_provider;
#[cfg(feature = "server")]
use crate::AuthSession;

#[cfg(feature = "server")]
use super::server_error;

#[cfg(feature = "server")]
fn candidate_from_album(album: &shared::metadata::AlbumWithTracks) -> LibraryTagCandidate {
    LibraryTagCandidate {
        release_id: album.album.id.clone(),
        title: album.album.title.clone(),
        artist: album.album.artist.clone(),
        year: album
            .album
            .release_date
            .as_deref()
            .and_then(|date| date.get(..4))
            .and_then(|year| year.parse().ok()),
        cover_url: album.album.cover_url.clone(),
        track_count: album.tracks.len() as u32,
        tracks: album
            .tracks
            .iter()
            .map(|track| LibraryTagTrack {
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: album.album.title.clone(),
                album_artist: album.album.artist.clone(),
                track_number: track.track_number,
                disc_number: track.disc_number,
                year: album
                    .album
                    .release_date
                    .as_deref()
                    .and_then(|date| date.get(..4))
                    .and_then(|year| year.parse().ok()),
                mb_track_id: track.mbid.clone(),
                mb_album_id: track.release_mbid.clone().or(album.album.mbid.clone()),
            })
            .collect(),
    }
}

#[cfg(feature = "server")]
fn candidate_from_track(track: &Track) -> LibraryTagCandidate {
    LibraryTagCandidate {
        release_id: track.id.clone(),
        title: track
            .album_title
            .clone()
            .unwrap_or_else(|| track.title.clone()),
        artist: track.artist.clone(),
        year: track
            .release_date
            .as_deref()
            .and_then(|date| date.get(..4))
            .and_then(|year| year.parse().ok()),
        cover_url: track
            .release_mbid
            .as_ref()
            .map(|mbid| format!("https://coverartarchive.org/release/{mbid}/front-250")),
        track_count: 1,
        tracks: vec![LibraryTagTrack {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album_title.clone().unwrap_or_default(),
            album_artist: track.artist.clone(),
            track_number: track.track_number,
            disc_number: track.disc_number,
            year: track
                .release_date
                .as_deref()
                .and_then(|date| date.get(..4))
                .and_then(|year| year.parse().ok()),
            mb_track_id: track.mbid.clone(),
            mb_album_id: track.release_mbid.clone(),
        }],
    }
}

#[cfg(feature = "server")]
fn token_overlap(left: &str, right: &str) -> f64 {
    let left: std::collections::HashSet<&str> = left.split_whitespace().collect();
    let right: std::collections::HashSet<&str> = right.split_whitespace().collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    left.intersection(&right).count() as f64 / left.union(&right).count() as f64
}

#[cfg(feature = "server")]
fn title_similarity(candidate: &str, file_title: &str) -> f64 {
    let candidate_key = normalize_key(candidate);
    let file_key = normalize_key(file_title);
    if candidate_key.is_empty() || file_key.is_empty() {
        return 0.0;
    }
    if candidate_key == file_key {
        return 1.0;
    }
    if candidate_key.contains(&file_key) || file_key.contains(&candidate_key) {
        return 0.75;
    }
    token_overlap(&candidate.to_lowercase(), &file_title.to_lowercase()) * 0.5
}

#[cfg(feature = "server")]
fn best_track_index(
    file_title: &str,
    file_number: Option<u32>,
    tracks: &[LibraryTagTrack],
) -> Option<usize> {
    tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let mut score = title_similarity(&track.title, file_title);
            if let (Some(file_number), Some(track_number)) = (file_number, track.track_number) {
                if file_number == track_number {
                    score += 0.5;
                }
            }
            (index, score)
        })
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .filter(|(_, score)| *score >= 0.5)
        .map(|(index, _)| index)
}

#[cfg(feature = "server")]
fn clean_query_name(value: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in value.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    let lower = out.to_lowercase();
    let normalized = out.replace(['_', '.'], " ");
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    let disc_like = |token: &str| {
        let token = token.to_lowercase();
        (token.starts_with("cd") || token.starts_with("disc") || token.starts_with("disk"))
            && token.chars().any(|ch| ch.is_ascii_digit())
    };
    let keep = tokens
        .iter()
        .filter(|token| !disc_like(token))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    if keep.is_empty() && !tokens.is_empty() && !lower.is_empty() {
        normalized.trim().to_string()
    } else {
        keep
    }
}

#[post("/api/library/tag/preview", auth: AuthSession)]
pub async fn preview_library_tagging(
    request: LibraryTagPreviewRequest,
) -> Result<LibraryTagPreview, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let folder = Folder::get_by_id(&request.folder_id)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Library folder not found"))?;
        if folder.user_id != auth.0.sub {
            return Err(super::unauthorized_error("Library folder not found"));
        }

        let mut release_mbid = None;
        let (artist, album) = if request.is_track {
            (
                request.artist.clone().unwrap_or_default(),
                request.album.clone().unwrap_or_default(),
            )
        } else if let Some(release_id) = &request.release_id {
            let release = LibraryReleaseRow::get_for_user(release_id, &auth.0.sub)
                .await
                .map_err(server_error)?
                .ok_or_else(|| server_error("Release not found"))?;
            release_mbid = release
                .mb_album_id
                .clone()
                .filter(|value| !value.is_empty());
            (release.album_artist, release.album)
        } else {
            (
                request.artist.clone().unwrap_or_default(),
                request.album.clone().unwrap_or_default(),
            )
        };

        let settings = UserSettings::get(&auth.0.sub).await.map_err(server_error)?;
        let default_provider = settings
            .default_metadata_provider
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("musicbrainz");
        let mut provider_ids = vec![default_provider.to_string()];
        for (id, _) in crate::services::available_metadata_providers().await {
            if id != default_provider && id != "lastfm" {
                provider_ids.push(id);
            }
        }
        if default_provider != "lastfm"
            && settings
                .lastfm_api_key
                .as_deref()
                .is_some_and(|key| !key.is_empty())
        {
            provider_ids.push("lastfm".to_string());
        }

        let mut seen_providers = std::collections::HashSet::new();
        let mut providers = Vec::new();
        for id in provider_ids {
            if !seen_providers.insert(id.clone()) {
                continue;
            }
            if let Ok(provider) =
                metadata_provider(Some(&id), settings.lastfm_api_key.as_deref()).await
            {
                providers.push(provider);
            }
        }
        if providers.is_empty() {
            providers.push(
                metadata_provider(None, settings.lastfm_api_key.as_deref())
                    .await
                    .map_err(server_error)?,
            );
        }

        let artist_filter =
            (!artist.is_empty() && artist != "Unknown Artist").then_some(artist.as_str());
        let path = std::path::Path::new(&request.path);
        let mut variants: Vec<(Option<String>, String)> = Vec::new();
        let mut seen_variants = std::collections::HashSet::new();
        {
            let mut push = |artist: Option<&str>, query: &str| {
                let query = clean_query_name(query);
                let artist = artist
                    .map(clean_query_name)
                    .filter(|value| !value.is_empty() && value != "Unknown Artist");
                if query.is_empty() {
                    return;
                }
                let key = (artist, query);
                if seen_variants.insert(key.clone()) {
                    variants.push(key);
                }
            };

            let dir = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let parent = path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let grandparent = path
                .parent()
                .and_then(|parent| parent.parent())
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or_default();

            push(artist_filter, &album);
            push(None, &album);
            if !request.is_track {
                push(Some(parent), dir);
                push(Some(grandparent), parent);
                push(None, dir);
                push(None, parent);
                push(Some(grandparent), dir);
            }
        }

        let mut candidates = Vec::new();
        if let Some(mbid) = release_mbid {
            for provider in &providers {
                if let Ok(album) = provider.get_album(&mbid).await {
                    candidates.push(candidate_from_album(&album));
                    break;
                }
            }
        }
        for provider in providers {
            if !candidates.is_empty() {
                break;
            }
            for (artist_filter, query) in &variants {
                if request.is_track {
                    let results = provider
                        .search_tracks(artist_filter.as_deref(), query, 8)
                        .await
                        .unwrap_or_default();
                    for result in results {
                        if let SearchResult::Track(track) = result {
                            candidates.push(candidate_from_track(&track));
                        }
                    }
                } else {
                    let results = provider
                        .search_albums(
                            artist_filter.as_deref(),
                            query,
                            &SearchFilters::default(),
                            8,
                        )
                        .await
                        .unwrap_or_default();
                    let mut editions = Vec::new();
                    for result in results {
                        match result {
                            SearchResult::Album(album) => editions.push(album),
                            SearchResult::AlbumGroup(group) => editions.extend(group.editions),
                            SearchResult::Track(_) => {}
                        }
                        if editions.len() >= 6 {
                            break;
                        }
                    }
                    for edition in editions.into_iter().take(4) {
                        match provider.get_album(&edition.id).await {
                            Ok(album) => candidates.push(candidate_from_album(&album)),
                            Err(e) => {
                                dioxus::logger::tracing::info!(
                                    "Tag preview could not load album {}: {}",
                                    edition.id,
                                    e
                                );
                            }
                        }
                    }
                }
                if !candidates.is_empty() {
                    break;
                }
            }
            if !candidates.is_empty() {
                break;
            }
        }

        let mut seen_candidates = std::collections::HashSet::new();
        candidates.retain(|candidate| seen_candidates.insert(candidate.release_id.clone()));

        if candidates.is_empty() {
            dioxus::logger::tracing::info!(
                "Tag preview found no metadata candidates for '{}' / '{}' at {} (queries: {:?})",
                artist,
                album,
                request.path,
                variants
                    .iter()
                    .map(|(artist, query)| (artist.clone(), query.clone()))
                    .collect::<Vec<_>>()
            );
        }

        let indexed_files = if request.is_track {
            None
        } else if let Some(release_id) = &request.release_id {
            Some(
                LibraryTrackRow::list_for_release(release_id)
                    .await
                    .map(|tracks| {
                        tracks
                            .into_iter()
                            .map(|track| std::path::PathBuf::from(track.path))
                            .filter(|path| path.is_file())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            )
        } else {
            None
        };
        let files: Vec<std::path::PathBuf> = if request.is_track {
            vec![path.to_path_buf()]
        } else if let Some(indexed_files) = indexed_files {
            indexed_files
        } else {
            soulbeet::library::audio_files_under(path)
        };
        let selected_candidate = 0.min(candidates.len().saturating_sub(1));
        let candidate_tracks = candidates
            .get(selected_candidate)
            .map(|candidate| candidate.tracks.as_slice())
            .unwrap_or(&[]);
        let files = files
            .into_iter()
            .map(|file| {
                let (number, title) = soulbeet::library::guess_file_title(&file);
                LibraryTagFile {
                    path: file.to_string_lossy().to_string(),
                    name: file
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    current_title: title.clone(),
                    track_number: number,
                    selected: best_track_index(&title, number, candidate_tracks),
                }
            })
            .collect();

        Ok(LibraryTagPreview {
            candidates,
            files,
            selected_candidate,
        })
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Ok(LibraryTagPreview::default())
    }
}

#[post("/api/library/tag/apply", auth: AuthSession)]
pub async fn apply_library_tagging(
    request: LibraryTagApplyRequest,
) -> Result<LibraryTagApplyResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        use shared::library::LibraryTagApplyTrack;

        let folder = Folder::get_by_id(&request.folder_id)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Library folder not found"))?;
        if folder.user_id != auth.0.sub {
            return Err(super::unauthorized_error("Library folder not found"));
        }
        if request.tracks.is_empty() {
            return Err(server_error("No tracks selected for tagging"));
        }

        let root = std::path::PathBuf::from(&folder.path);
        let label = request
            .tracks
            .first()
            .map(|track| track.album.clone())
            .filter(|album| !album.trim().is_empty())
            .unwrap_or_else(|| folder.name.clone());
        let artist = request
            .tracks
            .first()
            .map(|track| track.album_artist.clone())
            .filter(|artist| !artist.trim().is_empty());
        let request_id = uuid::Uuid::new_v4().to_string();
        let context = DownloadRequestContext {
            key: format!("tag:{request_id}"),
            label: format!("Tagging: {label}"),
            artist,
            track_count: None,
            release: Default::default(),
        };
        let row = DownloadRequestRow::create(
            &request_id,
            &auth.0.sub,
            &context,
            &folder.path,
            Some("beets"),
        )
        .await
        .map_err(server_error)?;

        let mut entries = Vec::new();
        for track in &request.tracks {
            let path = std::path::Path::new(&track.path);
            let size = tokio::fs::metadata(path)
                .await
                .map(|meta| meta.len())
                .unwrap_or(0);
            let item = DownloadableItem {
                id: track.path.clone(),
                source: "beets".to_string(),
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                size: Some(size),
                duration: None,
                quality: path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .to_lowercase(),
                quality_score: 0.0,
                backend_data: None,
            };
            let _ = DownloadRequestTrackRow::insert_if_new(&row.id, &item).await;
            entries.push(tag_progress(
                &row,
                &track.path,
                size,
                DownloadState::Importing,
                None,
            ));
        }

        let (tx, _) = get_or_create_user_channel(&auth.0.username).await;
        broadcast_progress(&tx, entries).await;

        let folder_id = request.folder_id.clone();
        let user_id = auth.0.sub.clone();
        let username = auth.0.username.clone();
        let tracks: Vec<LibraryTagApplyTrack> = request.tracks;
        tokio::spawn(async move {
            run_tagging_job(row, root, folder_id, user_id, username, tracks, tx).await;
        });

        Ok(LibraryTagApplyResult {
            beets_available: true,
            started: true,
            ..Default::default()
        })
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Ok(LibraryTagApplyResult::default())
    }
}

#[cfg(feature = "server")]
fn tag_progress(
    request: &DownloadRequestRow,
    path: &str,
    size: u64,
    state: DownloadState,
    error: Option<String>,
) -> DownloadProgress {
    let imported = matches!(state, DownloadState::Imported);
    DownloadProgress {
        id: path.to_string(),
        source: "beets".to_string(),
        item: path.to_string(),
        size,
        transferred: if imported { size } else { 0 },
        state,
        percent: if imported { 100.0 } else { 0.0 },
        speed: 0.0,
        error,
        backend: Some("beets".to_string()),
        batch_id: Some(request.id.clone()),
        batch_label: Some(request.label.clone()),
        local_path: None,
        import_output: None,
        retryable: false,
        can_choose_import: false,
    }
}

#[cfg(feature = "server")]
async fn run_tagging_job(
    request: DownloadRequestRow,
    root: std::path::PathBuf,
    folder_id: String,
    user_id: String,
    username: String,
    tracks: Vec<shared::library::LibraryTagApplyTrack>,
    tx: tokio::sync::broadcast::Sender<shared::download::DownloadEvent>,
) {
    use dioxus::logger::tracing::warn;

    let known = soulbeet::library::beets_known_paths(&root)
        .await
        .unwrap_or_default();
    let mut updated = 0u32;
    let mut failures = 0u32;

    for track in &tracks {
        let path = std::path::Path::new(&track.path);
        let size = tokio::fs::metadata(path)
            .await
            .map(|meta| meta.len())
            .unwrap_or(0);
        if !path.starts_with(&root) || !path.is_file() {
            failures += 1;
            broadcast_progress(
                &tx,
                vec![tag_progress(
                    &request,
                    &track.path,
                    size,
                    DownloadState::Failed("File is no longer in the library".to_string()),
                    Some("File is no longer in the library".to_string()),
                )],
            )
            .await;
            continue;
        }
        let path_key = tokio::fs::canonicalize(path)
            .await
            .unwrap_or_else(|_| path.to_path_buf());
        if !known.contains(&path_key) {
            if let Err(e) = soulbeet::library::import_as_is(&root, path).await {
                failures += 1;
                let error = format!("beets import failed: {e}");
                broadcast_progress(
                    &tx,
                    vec![tag_progress(
                        &request,
                        &track.path,
                        size,
                        DownloadState::Failed(error.clone()),
                        Some(error),
                    )],
                )
                .await;
                continue;
            }
        }
        let fields = vec![
            ("title", track.title.clone()),
            ("artist", track.artist.clone()),
            ("album", track.album.clone()),
            ("albumartist", track.album_artist.clone()),
            (
                "year",
                track.year.map(|year| year.to_string()).unwrap_or_default(),
            ),
            (
                "track",
                track
                    .track_number
                    .map(|number| number.to_string())
                    .unwrap_or_default(),
            ),
            (
                "disc",
                track
                    .disc_number
                    .map(|number| number.to_string())
                    .unwrap_or_default(),
            ),
            ("mb_trackid", track.mb_track_id.clone().unwrap_or_default()),
            ("mb_albumid", track.mb_album_id.clone().unwrap_or_default()),
        ];
        match soulbeet::library::modify_track(&root, path, &fields).await {
            Ok(()) => {
                updated += 1;
                broadcast_progress(
                    &tx,
                    vec![tag_progress(
                        &request,
                        &track.path,
                        size,
                        DownloadState::Imported,
                        None,
                    )],
                )
                .await;
            }
            Err(e) => {
                failures += 1;
                let error = format!("beets modify failed: {e}");
                warn!("Tagging failed for '{}': {}", track.path, e);
                broadcast_progress(
                    &tx,
                    vec![tag_progress(
                        &request,
                        &track.path,
                        size,
                        DownloadState::Failed(error.clone()),
                        Some(error),
                    )],
                )
                .await;
            }
        }
    }

    let _ = DownloadRequestRow::mark_completed_if_idle(&request.id).await;
    if failures > 0 {
        let _ = DownloadRequestRow::set_state(&request.id, "stopped").await;
    }

    if updated > 0 {
        let _ = crate::library::scan_folder_internal(&folder_id, true).await;
        crate::server_fns::media_server::trigger_media_scan(&user_id, &root).await;
    }

    if failures > 0 {
        dioxus::logger::tracing::info!(
            "Tagging job for {} finished for {} with {} failures",
            username,
            request.label,
            failures
        );
    } else {
        dioxus::logger::tracing::info!(
            "Tagging job for {} finished for {}",
            username,
            request.label
        );
    }
}
