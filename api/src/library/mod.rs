use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Once};
use std::time::Duration;

use dioxus::logger::tracing::{info, warn};
use shared::library::{LibraryScanInterval, LibraryScanSettings};
use tokio::sync::Mutex as AsyncMutex;

use crate::db::DB;
use crate::models::app_config::{keys, AppConfig};
use crate::models::folder::Folder;
use crate::models::library::{LibraryReleaseRow, LibraryScanStateRow};

#[cfg(feature = "server")]
pub mod quality;
pub mod watcher;

static SCAN_LOCKS: LazyLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Releases already fetched from the metadata source, by release MBID.
static ALBUM_CACHE: LazyLock<Mutex<HashMap<String, Arc<shared::metadata::AlbumWithTracks>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const ALBUM_CACHE_SIZE: usize = 128;

static TASKS_STARTED: Once = Once::new();

fn scan_lock(folder_id: &str) -> Arc<AsyncMutex<()>> {
    let mut locks = SCAN_LOCKS.lock().expect("scan lock registry poisoned");
    locks
        .entry(folder_id.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

#[derive(Debug, Clone, Default)]
pub struct ScanOutcome {
    pub release_count: usize,
    pub track_count: usize,
    pub warnings: Vec<String>,
}

pub async fn scan_folder_internal(
    folder_id: &str,
    sync_beets: bool,
) -> Result<ScanOutcome, String> {
    let lock = scan_lock(folder_id);
    let _guard = lock.lock().await;

    let folder = Folder::get_by_id(folder_id)
        .await?
        .ok_or_else(|| "Library folder not found".to_string())?;

    LibraryScanStateRow::set_status(folder_id, "scanning", None, None, 0, 0).await?;

    let root = PathBuf::from(&folder.path);
    match soulbeet::library::scan_library(&root, sync_beets).await {
        Ok(mut scan) => {
            #[cfg(feature = "server")]
            quality::apply(&root, &mut scan).await;
            let (release_count, track_count) =
                LibraryReleaseRow::replace_folder(folder_id, &folder.path, &scan).await?;
            if let Err(e) = resolve_main_artists(folder_id).await {
                warn!("Could not resolve main artists of '{}': {}", folder.name, e);
            }
            LibraryScanStateRow::set_status(
                folder_id,
                "idle",
                None,
                Some(&chrono::Utc::now().to_rfc3339()),
                release_count as i64,
                track_count as i64,
            )
            .await?;
            info!(
                "Library scan for '{}' indexed {} releases / {} tracks",
                folder.name, release_count, track_count
            );
            Ok(ScanOutcome {
                release_count,
                track_count,
                warnings: scan.warnings,
            })
        }
        Err(e) => {
            warn!("Library scan failed for '{}': {}", folder.name, e);
            LibraryScanStateRow::set_status(folder_id, "error", Some(&e), None, 0, 0).await?;
            Err(e)
        }
    }
}

pub async fn scan_folder_containing(path: &Path) -> Option<()> {
    let mut folders = all_folders().await;
    folders.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
    for folder in folders {
        if path.starts_with(Path::new(&folder.path)) {
            let _ = scan_folder_internal(&folder.id, false).await;
            return Some(());
        }
    }
    None
}

pub async fn scan_user_folders(user_id: &str, sync_beets: bool) -> Result<ScanOutcome, String> {
    let folders = Folder::get_all_by_user(user_id).await?;
    let mut total = ScanOutcome::default();
    for folder in folders {
        match scan_folder_internal(&folder.id, sync_beets).await {
            Ok(outcome) => {
                total.release_count += outcome.release_count;
                total.track_count += outcome.track_count;
                total.warnings.extend(outcome.warnings);
            }
            Err(e) => total.warnings.push(format!("{}: {}", folder.name, e)),
        }
    }
    Ok(total)
}

pub async fn scan_settings() -> LibraryScanSettings {
    let interval = AppConfig::get(keys::LIBRARY_SCAN_INTERVAL)
        .await
        .ok()
        .flatten()
        .map(|value| LibraryScanInterval::from_key(&value))
        .unwrap_or_default();
    let watch_enabled = AppConfig::get(keys::LIBRARY_WATCH_ENABLED)
        .await
        .ok()
        .flatten()
        .map(|value| matches!(value.trim().to_lowercase().as_str(), "true" | "1" | "yes"))
        .unwrap_or(false);
    let request_retry_interval = AppConfig::get(keys::LIBRARY_REQUEST_RETRY_INTERVAL)
        .await
        .ok()
        .flatten()
        .map(|value| LibraryScanInterval::from_key(&value))
        .unwrap_or_default();
    LibraryScanSettings {
        scan_interval: interval,
        watch_enabled,
        request_retry_interval,
    }
}

pub async fn save_scan_settings(settings: &LibraryScanSettings) -> Result<(), String> {
    AppConfig::set(keys::LIBRARY_SCAN_INTERVAL, settings.scan_interval.key()).await?;
    AppConfig::set(
        keys::LIBRARY_WATCH_ENABLED,
        if settings.watch_enabled {
            "true"
        } else {
            "false"
        },
    )
    .await?;
    AppConfig::set(
        keys::LIBRARY_REQUEST_RETRY_INTERVAL,
        settings.request_retry_interval.key(),
    )
    .await?;
    Ok(())
}

pub fn start_library_tasks() {
    TASKS_STARTED.call_once(|| {
        tokio::spawn(periodic_scan_loop());
        tokio::spawn(watcher::watch_loop());
        crate::request_retry::start_request_retry_task();
        info!("Started library scan and file watcher tasks");
    });
}

async fn all_folders() -> Vec<Folder> {
    sqlx::query_as::<_, Folder>("SELECT * FROM folders ORDER BY user_id, name")
        .fetch_all(&*crate::db::DB)
        .await
        .unwrap_or_default()
}

fn last_scan_elapsed(last: Option<&str>) -> Option<Duration> {
    let last = last?;
    let parsed = chrono::DateTime::parse_from_rfc3339(last)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(last, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|value| value.and_utc())
        })?;
    let elapsed = chrono::Utc::now().signed_duration_since(parsed);
    elapsed.to_std().ok()
}

async fn periodic_scan_loop() {
    tokio::time::sleep(Duration::from_secs(20)).await;
    let _ = sqlx::query("UPDATE library_scan_state SET status = 'idle' WHERE status = 'scanning'")
        .execute(&*crate::db::DB)
        .await;
    loop {
        let settings = scan_settings().await;
        let folders = all_folders().await;
        for folder in folders {
            let state = LibraryScanStateRow::get(&folder.id).await.ok().flatten();
            let elapsed =
                last_scan_elapsed(state.as_ref().and_then(|s| s.last_scanned_at.as_deref()));
            let due = match elapsed {
                None => true,
                Some(elapsed) => settings
                    .scan_interval
                    .duration()
                    .map(|interval| elapsed >= interval)
                    .unwrap_or(false),
            };
            if !due {
                continue;
            }
            if let Err(e) = scan_folder_internal(&folder.id, true).await {
                info!("Scheduled library scan failed for '{}': {}", folder.name, e);
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// What the metadata source knows about a release the scan cannot.
pub struct ResolvedRelease {
    pub expected: u32,
    pub main_artist: Option<String>,
    pub missing: Vec<shared::library::LibraryMissingTrack>,
}

/// Whether a credit names more than one artist, as `A feat. B` or `A & B`
/// do. Those are the releases whose grouping artist MusicBrainz can correct.
fn names_several_artists(artist: &str) -> bool {
    let artist = artist.to_lowercase();
    [" feat. ", " feat ", " ft. ", " ft ", " & ", ", ", " x ", " vs. ", " vs "]
        .iter()
        .any(|marker| artist.contains(marker))
}

pub async fn expected_tracks(
    release: &LibraryReleaseRow,
    existing_tracks: &[shared::library::LibraryTrackInfo],
) -> Option<ResolvedRelease> {
    let mbid = release
        .mb_album_id
        .as_deref()
        .filter(|value| !value.is_empty())?;

    let album = album_for(&release.id, mbid).await?;

    let expected = album.tracks.len() as u32;
    if release.expected_track_count != Some(expected as i64) {
        let _ = LibraryReleaseRow::set_expected_track_count(&release.id, expected).await;
    }

    let main_artist = album
        .main_artist
        .clone()
        .filter(|artist| artist != &release.album_artist);
    if let Some(artist) = main_artist.as_deref() {
        let _ = LibraryReleaseRow::set_main_artist(&release.id, artist).await;
    }

    let present_titles: std::collections::HashSet<String> = existing_tracks
        .iter()
        .map(|track| shared::library::normalize_key(&track.title))
        .collect();
    let present_mbids: std::collections::HashSet<String> = existing_tracks
        .iter()
        .filter_map(|track| track.mb_track_id.as_deref())
        .map(|mbid| mbid.trim().to_lowercase())
        .filter(|mbid| !mbid.is_empty())
        .collect();
    let missing = album
        .tracks
        .iter()
        .filter(|track| {
            let mbid_match = track
                .mbid
                .as_deref()
                .map(|mbid| mbid.trim().to_lowercase())
                .is_some_and(|mbid| !mbid.is_empty() && present_mbids.contains(&mbid))
                || {
                    let id = track.id.trim().to_lowercase();
                    !id.is_empty() && present_mbids.contains(&id)
                };
            !mbid_match && !present_titles.contains(&shared::library::normalize_key(&track.title))
        })
        .map(|track| shared::library::LibraryMissingTrack {
            disc_number: track.disc_number,
            track_number: track.track_number,
            title: track.title.clone(),
            artist: track.artist.clone(),
        })
        .collect();
    Some(ResolvedRelease {
        expected,
        main_artist,
        missing,
    })
}

/// The release as the metadata source describes it, the first provider that
/// knows it winning. Expanding the same release again is a hit, so only the
/// first look-up pays for a provider round trip.
async fn album_for(release_id: &str, mbid: &str) -> Option<Arc<shared::metadata::AlbumWithTracks>> {
    if let Some(cached) = ALBUM_CACHE
        .lock()
        .expect("album cache poisoned")
        .get(mbid)
        .cloned()
    {
        return Some(cached);
    }
    let mut provider_ids = vec!["musicbrainz".to_string()];
    for (id, _) in crate::services::available_metadata_providers().await {
        if id != "musicbrainz" && id != "lastfm" {
            provider_ids.push(id);
        }
    }
    for id in provider_ids {
        let Ok(provider) = crate::services::metadata_provider(Some(&id), None).await else {
            continue;
        };
        match provider.get_album(mbid).await {
            Ok(found) => {
                let album = Arc::new(found);
                let mut cache = ALBUM_CACHE.lock().expect("album cache poisoned");
                if cache.len() >= ALBUM_CACHE_SIZE {
                    cache.clear();
                }
                cache.insert(mbid.to_string(), Arc::clone(&album));
                return Some(album);
            }
            Err(e) => {
                info!(
                    "Provider '{}' could not resolve release '{}' of '{}': {}",
                    id, mbid, release_id, e
                );
            }
        }
    }
    None
}

/// Fills in the main artist of the releases the scan can only name with their
/// whole credit, so the library groups them under the act they belong to.
pub async fn resolve_main_artists(folder_id: &str) -> Result<(), String> {
    let releases = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT id, album_artist, mb_album_id FROM library_releases
         WHERE folder_id = ? AND mb_album_artist IS NULL",
    )
    .bind(folder_id)
    .fetch_all(&*DB)
    .await
    .map_err(|e| e.to_string())?;

    for (id, artist, mbid) in releases {
        if !names_several_artists(&artist) {
            continue;
        }
        let Some(mbid) = mbid.filter(|value| !value.is_empty()) else {
            continue;
        };
        let Some(album) = album_for(&id, &mbid).await else {
            continue;
        };
        let Some(main_artist) = album.main_artist.clone() else {
            continue;
        };
        if main_artist == artist {
            continue;
        }
        if let Err(e) = LibraryReleaseRow::set_main_artist(&id, &main_artist).await {
            warn!("Could not record the main artist of '{}': {}", id, e);
        }
    }
    Ok(())
}

