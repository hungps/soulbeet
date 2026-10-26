//! Force import: places the files a download kept into the library, tagged
//! the way the user mapped them onto the release they picked.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use shared::download::{DownloadProgress, DownloadState};
use shared::download::{DownloadTagApplyRequest, DownloadTagPreviewRequest};
use shared::library::LibraryTagApplyResult;
#[cfg(feature = "server")]
use shared::library::LibraryTagApplyTrack;
use shared::library::LibraryTagPreview;
#[cfg(feature = "server")]
use std::collections::{HashMap, HashSet};
#[cfg(feature = "server")]
use std::path::{Path, PathBuf};

#[cfg(feature = "server")]
use crate::globals::get_or_create_user_channel;
#[cfg(feature = "server")]
use crate::import_queue::{ImportJob, JobOwner};
#[cfg(feature = "server")]
use crate::models::download_request::{DownloadRequestRow, DownloadRequestTrackRow};
#[cfg(feature = "server")]
use crate::models::folder::Folder;
#[cfg(feature = "server")]
use crate::server_fns::download::requests::broadcast_progress;
#[cfg(feature = "server")]
use crate::server_fns::download::utils::{file_id, resolve_in_destination, FileId};
#[cfg(feature = "server")]
use crate::server_fns::library_tag::{tag_candidates, tag_files, tag_library_file};
#[cfg(feature = "server")]
use crate::server_fns::{refusal_error, server_error, unauthorized_error};
#[cfg(feature = "server")]
use crate::AuthSession;

/// The audio files a kept job still holds: the batch folder's, or the one
/// file of a singleton import.
#[cfg(feature = "server")]
fn job_files(job: &ImportJob) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for source in &job.sources {
        if source.is_dir() {
            files.extend(soulbeet::library::audio_files_under(source));
        } else if source.is_file() {
            files.push(source.clone());
        }
    }
    files.sort();
    files
}

/// Which row each file on disk belongs to, resolved the way the importer did.
#[cfg(feature = "server")]
fn file_items(job: &ImportJob, rows: &[DownloadProgress]) -> HashMap<PathBuf, String> {
    let mut pairs = HashMap::new();
    let Some(folder) = job.sources.first().filter(|source| source.is_dir()) else {
        if let (Some(source), Some(row)) = (job.sources.first(), rows.first()) {
            pairs.insert(source.clone(), row.item.clone());
        }
        return pairs;
    };
    let base = folder.parent().unwrap_or(folder);
    let destination = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut claimed: HashSet<FileId> = HashSet::new();
    for row in rows {
        let Some(path) = resolve_in_destination(&row.item, base, &destination, &claimed) else {
            continue;
        };
        if let Some(id) = file_id(&path) {
            claimed.insert(id);
        }
        pairs.insert(path, row.item.clone());
    }
    pairs
}

#[cfg(feature = "server")]
fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// The kept job of the row sent, with the row it keeps for that item.
#[cfg(feature = "server")]
async fn kept_job_and_row(
    username: &str,
    user_id: &str,
    item: &str,
) -> Result<
    (
        ImportJob,
        DownloadProgress,
        Option<DownloadRequestTrackRow>,
    ),
    ServerFnError,
> {
    let job = crate::import_queue::kept_job_for(username, item)
        .await
        .ok_or_else(|| refusal_error("Nothing is left to import for this row"))?;
    let row = job
        .owner
        .as_ref()
        .and_then(|owner| owner.rows.iter().find(|row| row.item == item))
        .cloned()
        .ok_or_else(|| server_error("This import has no row to tag"))?;
    let stored = DownloadRequestTrackRow::find_by_item(user_id, item)
        .await
        .map_err(server_error)?;
    Ok((job, row, stored))
}

/// The editions the kept files could be tagged with, and how each file maps
/// onto the selected one.
#[post("/api/downloads/tag/preview", auth: AuthSession)]
pub async fn preview_download_tagging(
    request: DownloadTagPreviewRequest,
) -> Result<LibraryTagPreview, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (job, row, stored) = kept_job_and_row(&auth.0.username, &auth.0.sub, &request.item).await?;
        let is_track = matches!(job.mode, soulbeet::ImportMode::Singleton);
        let album = stored
            .as_ref()
            .and_then(|stored| non_empty(&stored.album))
            .or_else(|| row.batch_label.clone())
            .unwrap_or_default();
        let artist = stored
            .as_ref()
            .map(|stored| stored.artist.clone())
            .unwrap_or_default();
        let path = job
            .sources
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&request.item));
        let candidates = tag_candidates(
            &auth.0.sub,
            &path,
            &artist,
            &album,
            is_track,
            request.release_mbid.clone(),
        )
        .await;
        let selected_candidate = 0.min(candidates.len().saturating_sub(1));
        let tracks = candidates
            .get(selected_candidate)
            .map(|candidate| candidate.tracks.as_slice())
            .unwrap_or(&[]);
        let rows = job
            .owner
            .as_ref()
            .map(|owner| owner.rows.as_slice())
            .unwrap_or(&[]);
        let items = file_items(&job, rows);
        let mut files = tag_files(job_files(&job), tracks);
        for file in &mut files {
            file.item = items.get(Path::new(&file.path)).cloned();
        }
        Ok(LibraryTagPreview {
            files,
            candidates,
            selected_candidate,
        })
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Ok(LibraryTagPreview::default())
    }
}

/// One path component of a tag, safe on disk and never `.` or `..`.
#[cfg(feature = "server")]
fn safe_component(value: &str) -> Option<String> {
    let name = shared::slskd::sanitize_filename(value);
    let name = name.trim().trim_matches('.');
    (!name.is_empty()).then(|| name.to_string())
}

/// Where a track belongs under `root`: `<albumartist>/<album>/<track> <title>`.
#[cfg(feature = "server")]
fn destination_path(root: &Path, track: &LibraryTagApplyTrack, source: &Path) -> PathBuf {
    let album_artist = safe_component(&track.album_artist)
        .or_else(|| safe_component(&track.artist))
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let album = safe_component(&track.album);
    let (number, guessed) = soulbeet::library::guess_file_title(source);
    let title = safe_component(&track.title)
        .or_else(|| safe_component(&guessed))
        .unwrap_or_else(|| "Unknown Track".to_string());
    let stem = match track.track_number.or(number) {
        Some(number) => format!("{number:02} {title}"),
        None => title,
    };
    let file_name = match source.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if !extension.is_empty() => format!("{stem}.{extension}"),
        _ => stem,
    };
    let file_name = safe_component(&file_name).unwrap_or_else(|| "Unknown Track".to_string());

    let mut directory = root.join(album_artist);
    if let Some(album) = album {
        directory = directory.join(album);
    }
    directory.join(file_name)
}

/// `path`, or the first free `<stem> (<n>)<ext>` next to it, so a copy never
/// overwrites a library file.
#[cfg(feature = "server")]
fn free_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("track");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    for n in 2..1000 {
        let name = if extension.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{extension}")
        };
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// The row's own state, with the fields a tagging run changes.
#[cfg(feature = "server")]
fn row_progress(
    template: &DownloadProgress,
    state: DownloadState,
    error: Option<String>,
) -> DownloadProgress {
    let imported = matches!(state, DownloadState::Imported);
    DownloadProgress {
        percent: if imported { 100.0 } else { template.percent },
        transferred: if imported {
            template.size
        } else {
            template.transferred
        },
        speed: 0.0,
        retryable: false,
        can_choose_import: false,
        state,
        error,
        ..template.clone()
    }
}

/// The row `item` stands for in this job, so a copied file reports on it.
#[cfg(feature = "server")]
fn parent_row<'a>(owner: &'a JobOwner, item: Option<&str>) -> Option<&'a DownloadProgress> {
    let rows = &owner.rows;
    item.and_then(|item| rows.iter().find(|row| row.item == item))
        .or_else(|| rows.first())
}

/// Copies the chosen files into the library and tags them there, reporting on
/// the row they came from.
#[post("/api/downloads/tag/apply", auth: AuthSession)]
pub async fn apply_download_tagging(
    request: DownloadTagApplyRequest,
) -> Result<LibraryTagApplyResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        if request.tracks.is_empty() {
            return Err(server_error("No tracks selected for tagging"));
        }
        let (job, row, _) = kept_job_and_row(&auth.0.username, &auth.0.sub, &request.item).await?;
        let request_id = row
            .batch_id
            .clone()
            .ok_or_else(|| server_error("This row belongs to no request"))?;
        if DownloadRequestRow::get_for_user(&request_id, &auth.0.sub)
            .await
            .map_err(server_error)?
            .is_none()
        {
            return Err(unauthorized_error("Request not found"));
        }

        let allowed = job_files(&job);
        for track in &request.tracks {
            if !allowed.iter().any(|file| file == Path::new(&track.path)) {
                return Err(refusal_error("That file is not part of this download"));
            }
        }

        let target = job.target.clone();
        let folder_id = Folder::find_for_user_by_path(&auth.0.sub, &target.to_string_lossy())
            .await
            .map_err(server_error)?
            .map(|folder| folder.id);
        let owner = job.owner.clone().unwrap_or(JobOwner {
            username: auth.0.username.clone(),
            rows: vec![row],
        });
        let user_id = auth.0.sub.clone();
        let username = auth.0.username.clone();
        let tracks = request.tracks;
        let (tx, _) = get_or_create_user_channel(&username).await;
        tokio::spawn(run_download_tagging(
            request_id, target, folder_id, user_id, username, owner, tracks, tx,
        ));

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
#[allow(clippy::too_many_arguments)]
async fn run_download_tagging(
    request_id: String,
    target: PathBuf,
    folder_id: Option<String>,
    user_id: String,
    username: String,
    owner: JobOwner,
    tracks: Vec<LibraryTagApplyTrack>,
    tx: tokio::sync::broadcast::Sender<shared::download::DownloadEvent>,
) {
    use dioxus::logger::tracing::{info, warn};

    let known = soulbeet::library::beets_known_paths(&target)
        .await
        .unwrap_or_default();
    let mut updated = 0u32;
    let mut failures = 0u32;

    // Mark the rows this run covers straight away, so the drawer shows BEETS
    // instead of the skip reason while the files are being placed.
    let mut started = Vec::new();
    let mut seen = HashSet::new();
    for track in &tracks {
        let Some(template) = parent_row(&owner, track.item.as_deref()) else {
            continue;
        };
        if !seen.insert(template.item.clone()) {
            continue;
        }
        let progress = row_progress(template, DownloadState::Importing, None);
        let _ = DownloadRequestTrackRow::update_progress(&request_id, &progress).await;
        started.push(progress);
    }
    broadcast_progress(&tx, started).await;

    for track in &tracks {
        let source = PathBuf::from(&track.path);
        let destination = free_path(&destination_path(&target, track, &source));
        let error = match copy_into_library(&source, &destination).await {
            Ok(()) => tag_library_file(&target, &destination, track, &known)
                .await
                .err(),
            Err(e) => Some(e),
        };
        let template = parent_row(&owner, track.item.as_deref());
        let progress = match error {
            Some(error) => {
                failures += 1;
                warn!("Force import of '{}' failed: {}", track.path, error);
                let template = template.cloned().unwrap_or_else(|| placeholder(&track.path));
                row_progress(&template, DownloadState::Failed(error.clone()), Some(error))
            }
            None => {
                updated += 1;
                let template = template.cloned().unwrap_or_else(|| placeholder(&track.path));
                row_progress(&template, DownloadState::Imported, None)
            }
        };
        let _ = DownloadRequestTrackRow::update_progress(&request_id, &progress).await;
        broadcast_progress(&tx, vec![progress]).await;
    }

    let items: Vec<String> = owner.rows.iter().map(|row| row.item.clone()).collect();
    crate::import_queue::forget_retries(&username, &items).await;

    let _ = DownloadRequestRow::mark_completed_if_idle(&request_id).await;
    if failures > 0 {
        let _ = DownloadRequestRow::set_state(&request_id, "stopped").await;
    }

    if updated > 0 {
        if let Some(folder_id) = folder_id {
            let _ = crate::library::scan_folder_internal(&folder_id, true).await;
        }
        crate::server_fns::media_server::trigger_media_scan(&user_id, &target).await;
    }

    info!("Force import for {username} finished with {updated} placed and {failures} failed");
}

#[cfg(feature = "server")]
fn placeholder(item: &str) -> DownloadProgress {
    DownloadProgress::queued(item.to_string(), String::new(), item.to_string(), 0)
}

#[cfg(feature = "server")]
async fn copy_into_library(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() {
        return Err("The downloaded file is gone".to_string());
    }
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Could not create the library folder: {e}"))?;
    }
    tokio::fs::copy(source, destination)
        .await
        .map(|_| ())
        .map_err(|e| format!("Could not copy the file into the library: {e}"))
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use crate::import_queue::JobOwner;
    use soulbeet::{DriverChoice, ImportMode};

    fn apply_track(title: &str, number: Option<u32>) -> LibraryTagApplyTrack {
        LibraryTagApplyTrack {
            path: "/dl/X/01.flac".to_string(),
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: "An Album".to_string(),
            album_artist: "Album Artist".to_string(),
            track_number: number,
            disc_number: None,
            year: None,
            mb_track_id: None,
            mb_album_id: None,
            item: Some("item".to_string()),
        }
    }

    fn row(item: &str) -> DownloadProgress {
        DownloadProgress {
            retryable: true,
            can_choose_import: true,
            ..DownloadProgress::queued(item.to_string(), "peer".to_string(), item.to_string(), 10)
        }
    }

    #[test]
    fn a_track_lands_under_its_album_artist_and_album() {
        let path = destination_path(
            Path::new("/music"),
            &apply_track("A Song", Some(3)),
            Path::new("/dl/X/01.flac"),
        );
        assert_eq!(path, Path::new("/music/Album Artist/An Album/03 A Song.flac"));

        let without_number = destination_path(
            Path::new("/music"),
            &apply_track("A Song", None),
            Path::new("/dl/X/01.flac"),
        );
        // No number in the edition: the file's own number keeps it ordered.
        assert_eq!(
            without_number,
            Path::new("/music/Album Artist/An Album/01 A Song.flac")
        );
    }

    #[test]
    fn hostile_tag_names_cannot_leave_the_library() {
        let mut track = apply_track("../../A Song", Some(1));
        track.album_artist = "AC/DC".to_string();
        track.album = "..".to_string();
        let path = destination_path(Path::new("/music"), &track, Path::new("/dl/X/01.flac"));
        assert_eq!(path, Path::new("/music/AC_DC/01 _.._A Song.flac"));
        assert!(path.components().all(|part| part != std::path::Component::ParentDir));
    }

    #[test]
    fn a_copy_never_overwrites_an_existing_library_file() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("An Album").join("01 A Song.flac");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();

        assert_eq!(free_path(&target), target);
        std::fs::write(&target, b"x").unwrap();
        assert_eq!(
            free_path(&target),
            target.with_file_name("01 A Song (2).flac")
        );
        std::fs::write(target.with_file_name("01 A Song (2).flac"), b"x").unwrap();
        assert_eq!(
            free_path(&target),
            target.with_file_name("01 A Song (3).flac")
        );
    }

    #[test]
    fn a_placed_row_drops_its_retry_offers() {
        let template = row("music\\A\\01.flac");
        let placed = row_progress(&template, DownloadState::Imported, None);
        assert_eq!(placed.item, template.item);
        assert_eq!(placed.state, DownloadState::Imported);
        assert!(!placed.retryable && !placed.can_choose_import);
        assert_eq!(placed.percent, 100.0);

        let failed = row_progress(&template, DownloadState::Failed("x".into()), Some("x".into()));
        assert_eq!(failed.percent, template.percent);
        assert!(!failed.retryable && !failed.can_choose_import);
    }

    fn owned_job(
        sources: Vec<PathBuf>,
        mode: ImportMode,
        rows: Vec<DownloadProgress>,
    ) -> ImportJob {
        ImportJob {
            sources,
            target: PathBuf::from("/music"),
            mode,
            owner: Some(JobOwner {
                username: "u".to_string(),
                rows,
            }),
            snapshot_target: false,
            cleanup_root: None,
            choice: DriverChoice::Auto,
            choices: None,
        }
    }

    #[test]
    fn each_file_reports_on_the_row_it_was_downloaded_for() {
        let tmp = tempfile::tempdir().unwrap();
        let batch = tmp.path().join("X [12345678]");
        std::fs::create_dir_all(&batch).unwrap();
        std::fs::write(batch.join("01.flac"), b"one").unwrap();
        std::fs::write(batch.join("02.flac"), b"two").unwrap();

        let job = owned_job(
            vec![batch.clone()],
            ImportMode::Album,
            vec![row(r"music\A\02.flac"), row(r"music\A\01.flac")],
        );
        let rows = job.owner.as_ref().unwrap().rows.clone();
        let pairs = file_items(&job, &rows);

        assert_eq!(
            pairs.get(&batch.join("01.flac")).map(String::as_str),
            Some(r"music\A\01.flac")
        );
        assert_eq!(
            pairs.get(&batch.join("02.flac")).map(String::as_str),
            Some(r"music\A\02.flac")
        );

        let single = owned_job(
            vec![batch.join("01.flac")],
            ImportMode::Singleton,
            vec![row("a.flac")],
        );
        let rows = single.owner.as_ref().unwrap().rows.clone();
        assert_eq!(
            file_items(&single, &rows).get(&batch.join("01.flac")).map(String::as_str),
            Some("a.flac")
        );
    }
}
