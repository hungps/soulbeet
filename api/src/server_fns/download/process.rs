use dioxus::logger::tracing::{info, warn};
use shared::download::{DownloadProgress, DownloadReleaseInfo, DownloadState};
use soulbeet::{DriverChoice, ImportMode};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::utils::{file_id, resolve_download_path_mapped, resolve_in_destination, FileId};
use crate::import_queue::{ImportJob, JobOwner};

/// Maximum number of retries when waiting for a downloaded file to appear on disk.
/// With exponential backoff (500ms, 1s, 2s, 4s, 8s), this covers ~15.5s total.
const FILE_RESOLVE_MAX_RETRIES: u32 = 5;

/// Initial delay between file resolution retries.
const FILE_RESOLVE_INITIAL_DELAY_MS: u64 = 500;

/// Find a finished download on disk, retrying with exponential backoff:
/// slskd can report completion before the file is visible through a shared
/// volume mount. `destination` is the batch folder the file was enqueued
/// into; `None` (a transfer this batch did not enqueue) falls back to
/// searching the whole download folder. Files in `claimed` were already
/// handed off and are never returned; the file found is added to it. The
/// lock is held only while one attempt looks, never across a wait.
pub async fn resolve_on_disk(
    filename: &str,
    base: &Path,
    destination: Option<&str>,
    claimed: &Mutex<HashSet<FileId>>,
) -> Option<PathBuf> {
    let slskd_base = crate::models::app_config::AppConfig::slskd_download_path().await;
    let attempt = || {
        let mut claimed = claimed.lock().unwrap_or_else(|e| e.into_inner());
        let path = match destination {
            Some(destination) => resolve_in_destination(filename, base, destination, &claimed),
            None => resolve_download_path_mapped(filename, base, slskd_base.as_deref())
                .map(PathBuf::from),
        }?;
        let id = file_id(&path)?;
        claimed.insert(id).then_some(path)
    };
    if let Some(path) = attempt() {
        return Some(path);
    }

    let mut delay_ms = FILE_RESOLVE_INITIAL_DELAY_MS;
    for retry in 1..=FILE_RESOLVE_MAX_RETRIES {
        warn!(
            "File not found on disk (attempt {}), retrying in {}ms: {}",
            retry, delay_ms, filename
        );
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

        if let Some(path) = attempt() {
            info!(
                "File appeared on disk after {}ms wait: {}",
                delay_ms, filename
            );
            return Some(path);
        }

        delay_ms *= 2;
    }

    None
}

/// Row text for a finished download whose file was not found on disk.
const UNRESOLVED: &str = "Could not resolve file path";

/// The Failed row of a finished download whose file was not found.
pub fn unresolved_row(row: DownloadProgress) -> DownloadProgress {
    DownloadProgress {
        state: DownloadState::Failed(UNRESOLVED.into()),
        error: Some(UNRESOLVED.into()),
        ..row
    }
}

/// A finished slot whose file was found on disk. `destination` is the batch
/// folder it was enqueued into, `None` for an adopted transfer.
pub struct ResolvedSlot {
    pub row: DownloadProgress,
    pub path: PathBuf,
    pub destination: Option<String>,
}

/// beets' name for its MusicBrainz source.
const MUSICBRAINZ: &str = "MusicBrainz";

/// How the import of a requested release should match it: onto the release or
/// recording selected in search, when the request kept a MusicBrainz id for
/// it, and beets' own match otherwise. A release id is only used when it is
/// the album the request was keyed on, so a release group from another
/// provider is never handed to beets as a release.
pub fn import_choice(
    mode: ImportMode,
    release: &DownloadReleaseInfo,
    request_key: &str,
) -> DriverChoice {
    let id = match mode {
        ImportMode::Album => release
            .release_mbid
            .as_deref()
            .filter(|id| request_key == format!("album:{id}")),
        ImportMode::Singleton => release.recording_mbid.as_deref(),
    };
    match id.filter(|id| !id.is_empty()) {
        Some(id) => DriverChoice::Pick {
            data_source: MUSICBRAINZ.to_string(),
            id: id.to_string(),
        },
        None => DriverChoice::Auto,
    }
}

/// An import of `source` reporting to `username` through `rows`, which are
/// already stamped with their batch.
#[allow(clippy::too_many_arguments)]
pub fn import_job(
    source: PathBuf,
    mode: ImportMode,
    rows: Vec<DownloadProgress>,
    target: &Path,
    username: &str,
    base: &Path,
    release: &DownloadReleaseInfo,
    request_key: &str,
) -> ImportJob {
    ImportJob {
        sources: vec![source],
        target: target.to_path_buf(),
        mode,
        owner: Some(JobOwner {
            username: username.to_string(),
            rows,
        }),
        snapshot_target: false,
        cleanup_root: Some(base.to_path_buf()),
        choice: import_choice(mode, release, request_key),
        choices: None,
    }
}

/// The imports of an album batch: its folder as one album, and each file
/// found outside it (an adopted transfer) on its own, since that file's
/// folder may hold anything.
pub fn album_jobs(
    resolved: Vec<ResolvedSlot>,
    target: &Path,
    username: &str,
    base: &Path,
    release: &DownloadReleaseInfo,
    request_key: &str,
) -> Vec<ImportJob> {
    let mut albums: Vec<(PathBuf, Vec<DownloadProgress>)> = Vec::new();
    let mut jobs = Vec::new();
    for slot in resolved {
        match slot.destination {
            Some(destination) => {
                let folder = base.join(destination);
                match albums.iter_mut().find(|(f, _)| *f == folder) {
                    Some((_, rows)) => rows.push(slot.row),
                    None => albums.push((folder, vec![slot.row])),
                }
            }
            None => jobs.push(import_job(
                slot.path,
                ImportMode::Singleton,
                vec![slot.row],
                target,
                username,
                base,
                &DownloadReleaseInfo::default(),
                "",
            )),
        }
    }
    let album_jobs = albums.into_iter().map(|(folder, rows)| {
        import_job(
            folder,
            ImportMode::Album,
            rows,
            target,
            username,
            base,
            release,
            request_key,
        )
    });
    album_jobs.chain(jobs).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_queue::test_support::row;

    #[test]
    fn an_album_imports_its_batch_folder_once_and_adopted_files_alone() {
        let batch = Some("X [12345678]".to_string());
        let resolved = vec![
            ResolvedSlot {
                row: row("a"),
                path: PathBuf::from("/dl/X [12345678]/a.flac"),
                destination: batch.clone(),
            },
            ResolvedSlot {
                row: row("adopted"),
                path: PathBuf::from("/dl/Other/f.flac"),
                destination: None,
            },
            ResolvedSlot {
                row: row("b"),
                path: PathBuf::from("/dl/X [12345678]/b_639097129778484198.flac"),
                destination: batch,
            },
        ];

        let jobs: Vec<(Vec<PathBuf>, ImportMode, Vec<String>)> = album_jobs(
            resolved,
            Path::new("/music"),
            "u",
            Path::new("/dl"),
            &DownloadReleaseInfo::default(),
            "",
        )
        .into_iter()
        .map(|job| {
            assert_eq!(job.cleanup_root.as_deref(), Some(Path::new("/dl")));
            assert_eq!(job.target, Path::new("/music"));
            let owner = job.owner.unwrap();
            assert_eq!(owner.username, "u");
            let items = owner.rows.into_iter().map(|r| r.item).collect();
            (job.sources, job.mode, items)
        })
        .collect();

        assert_eq!(
            jobs,
            vec![
                (
                    vec![PathBuf::from("/dl/X [12345678]")],
                    ImportMode::Album,
                    vec!["a".to_string(), "b".to_string()]
                ),
                (
                    vec![PathBuf::from("/dl/Other/f.flac")],
                    ImportMode::Singleton,
                    vec!["adopted".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn an_album_job_carries_the_picked_release() {
        let resolved = vec![ResolvedSlot {
            row: row("a"),
            path: PathBuf::from("/dl/X [12345678]/a.flac"),
            destination: Some("X [12345678]".to_string()),
        }];

        let jobs = album_jobs(
            resolved,
            Path::new("/music"),
            "u",
            Path::new("/dl"),
            &release(Some("release-id"), None),
            "album:release-id",
        );

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].choice, picked("release-id"));
    }

    fn release(release_mbid: Option<&str>, recording_mbid: Option<&str>) -> DownloadReleaseInfo {
        DownloadReleaseInfo {
            release_mbid: release_mbid.map(str::to_string),
            recording_mbid: recording_mbid.map(str::to_string),
            ..DownloadReleaseInfo::default()
        }
    }

    fn picked(id: &str) -> DriverChoice {
        DriverChoice::Pick {
            data_source: MUSICBRAINZ.to_string(),
            id: id.to_string(),
        }
    }

    #[test]
    fn an_album_import_picks_the_release_the_request_was_keyed_on() {
        assert_eq!(
            import_choice(
                ImportMode::Album,
                &release(Some("release-id"), None),
                "album:release-id"
            ),
            picked("release-id")
        );
    }

    #[test]
    fn an_album_import_ignores_an_id_the_request_is_not_keyed_on() {
        // Another provider's id is a release group, not a release beets could
        // be pointed at.
        assert_eq!(
            import_choice(
                ImportMode::Album,
                &release(Some("release-group-id"), None),
                "album:lastfm:Artist:Album"
            ),
            DriverChoice::Auto
        );
    }

    #[test]
    fn an_album_import_without_a_release_id_matches_on_its_own() {
        assert_eq!(
            import_choice(ImportMode::Album, &DownloadReleaseInfo::default(), "album:x"),
            DriverChoice::Auto
        );
    }

    #[test]
    fn a_singleton_import_picks_the_recording_it_was_requested_for() {
        assert_eq!(
            import_choice(
                ImportMode::Singleton,
                &release(None, Some("recording-id")),
                "track:recording-id"
            ),
            picked("recording-id")
        );
    }

    #[test]
    fn a_singleton_import_without_a_recording_id_matches_on_its_own() {
        assert_eq!(
            import_choice(
                ImportMode::Singleton,
                &release(Some("release-id"), None),
                "track:x"
            ),
            DriverChoice::Auto
        );
    }

    #[test]
    fn an_empty_id_is_not_forced() {
        assert_eq!(
            import_choice(ImportMode::Album, &release(Some(""), None), "album:"),
            DriverChoice::Auto
        );
    }

    #[tokio::test]
    async fn resolve_on_disk_looks_in_the_batch_folder() {
        let base = tempfile::tempdir().unwrap();
        let folder = base.path().join("X [12345678]");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("01.flac"), b"x").unwrap();

        let found = resolve_on_disk(
            r"music\A\01.flac",
            base.path(),
            Some("X [12345678]"),
            &Mutex::new(HashSet::new()),
        )
        .await;
        assert_eq!(found, Some(folder.join("01.flac")));
    }

    #[tokio::test]
    async fn a_resolved_file_is_claimed_and_not_returned_twice() {
        let base = tempfile::tempdir().unwrap();
        let folder = base.path().join("X [12345678]");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("01.flac"), b"x").unwrap();
        std::fs::write(folder.join("01_639097129778484198.flac"), b"x").unwrap();
        let claimed = Mutex::new(HashSet::new());

        let first = resolve_on_disk("a/01.flac", base.path(), Some("X [12345678]"), &claimed);
        assert_eq!(first.await, Some(folder.join("01.flac")));
        let second = resolve_on_disk("b/01.flac", base.path(), Some("X [12345678]"), &claimed);
        assert_eq!(
            second.await,
            Some(folder.join("01_639097129778484198.flac"))
        );
        assert_eq!(claimed.lock().unwrap().len(), 2);
    }
}
