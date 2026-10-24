use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use shared::library::{
    DownloadActivity, LibraryFolderInfo, LibraryMatchQuery, LibraryMatchResult, LibraryOverview,
    LibraryRelease, LibraryReleaseDetail, LibraryRemovalResult, LibraryScanSettings,
};

#[cfg(feature = "server")]
use dioxus::logger::tracing::warn;

#[cfg(feature = "server")]
use crate::models::folder::Folder;
#[cfg(feature = "server")]
use crate::models::library::{LibraryReleaseRow, LibraryTrackRow};
#[cfg(feature = "server")]
use crate::AuthSession;

#[cfg(feature = "server")]
use super::{cleanup_empty_ancestors, server_error};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryScanResult {
    pub folder: LibraryFolderInfo,
    pub warnings: Vec<String>,
}

#[get("/api/library/overview", auth: AuthSession)]
pub async fn get_library_overview() -> Result<LibraryOverview, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let folders = LibraryReleaseRow::folder_infos(&auth.0.sub)
            .await
            .map_err(server_error)?;
        let releases = LibraryReleaseRow::list(&auth.0.sub, None)
            .await
            .map_err(server_error)?
            .iter()
            .map(LibraryReleaseRow::to_shared)
            .collect();
        Ok(LibraryOverview { folders, releases })
    }
    #[cfg(not(feature = "server"))]
    Ok(LibraryOverview::default())
}

#[get("/api/library/folders", auth: AuthSession)]
pub async fn get_library_folders() -> Result<Vec<LibraryFolderInfo>, ServerFnError> {
    #[cfg(feature = "server")]
    {
        LibraryReleaseRow::folder_infos(&auth.0.sub)
            .await
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    Ok(Vec::new())
}

#[post("/api/library/releases", auth: AuthSession)]
pub async fn get_library_releases(
    folder_id: Option<String>,
) -> Result<Vec<LibraryRelease>, ServerFnError> {
    #[cfg(feature = "server")]
    {
        LibraryReleaseRow::list(&auth.0.sub, folder_id.as_deref())
            .await
            .map(|rows| rows.iter().map(LibraryReleaseRow::to_shared).collect())
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = folder_id;
        Ok(Vec::new())
    }
}

#[post("/api/library/release", auth: AuthSession)]
pub async fn get_library_release_detail(
    release_id: String,
) -> Result<Option<LibraryReleaseDetail>, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let row = LibraryReleaseRow::get_for_user(&release_id, &auth.0.sub)
            .await
            .map_err(server_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let Some(mut detail) = LibraryReleaseRow::detail(&release_id, &auth.0.sub)
            .await
            .map_err(server_error)?
        else {
            return Ok(None);
        };
        if let Some((expected, missing)) =
            crate::library::expected_tracks(&row, &detail.tracks).await
        {
            detail.release.expected_track_count = Some(expected);
            if (!missing.is_empty() || expected > detail.release.track_count)
                && detail.release.matched_track_count > 0
            {
                detail.release.completeness = shared::library::ReleaseCompleteness::Partial;
            }
            detail.missing_tracks = missing;
        }
        Ok(Some(detail))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = release_id;
        Ok(None)
    }
}

#[post("/api/library/scan", auth: AuthSession)]
pub async fn scan_library_folder(folder_id: String) -> Result<LibraryScanResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let folder = Folder::get_by_id(&folder_id)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Library folder not found"))?;
        if folder.user_id != auth.0.sub {
            return Err(super::unauthorized_error("Library folder not found"));
        }
        let outcome = crate::library::scan_folder_internal(&folder_id, true)
            .await
            .map_err(server_error)?;
        let folders = LibraryReleaseRow::folder_infos(&auth.0.sub)
            .await
            .map_err(server_error)?;
        let info = folders
            .into_iter()
            .find(|info| info.id == folder_id)
            .unwrap_or_default();
        Ok(LibraryScanResult {
            folder: info,
            warnings: outcome.warnings,
        })
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = folder_id;
        Ok(LibraryScanResult::default())
    }
}

#[post("/api/library/scan-all", auth: AuthSession)]
pub async fn scan_all_library_folders() -> Result<LibraryScanResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let outcome = crate::library::scan_user_folders(&auth.0.sub, true)
            .await
            .map_err(server_error)?;
        let total = LibraryFolderInfo {
            id: String::new(),
            name: "All folders".to_string(),
            path: String::new(),
            release_count: outcome.release_count as u32,
            track_count: outcome.track_count as u32,
            ..Default::default()
        };
        Ok(LibraryScanResult {
            folder: total,
            warnings: outcome.warnings,
        })
    }
    #[cfg(not(feature = "server"))]
    Ok(LibraryScanResult::default())
}

#[post("/api/library/remove-release", auth: AuthSession)]
pub async fn remove_library_release(
    release_id: String,
) -> Result<LibraryRemovalResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let release = LibraryReleaseRow::get_for_user(&release_id, &auth.0.sub)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Release not found"))?;
        let folder = Folder::get_by_id(&release.folder_id)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Library folder not found"))?;
        let tracks = LibraryTrackRow::list_for_release(&release_id)
            .await
            .map_err(server_error)?;
        let paths: Vec<String> = tracks.iter().map(|track| track.path.clone()).collect();
        let has_nested = LibraryReleaseRow::has_nested_releases(
            &release_id,
            &release.folder_id,
            &release.release_path,
        )
        .await
        .unwrap_or(true);

        let result = remove_paths(&folder.path, &release.release_path, &paths, !has_nested).await;
        LibraryReleaseRow::delete(&release_id)
            .await
            .map_err(server_error)?;

        crate::server_fns::media_server::trigger_media_scan(
            &auth.0.sub,
            std::path::Path::new(&folder.path),
        )
        .await;

        Ok(result)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = release_id;
        Ok(LibraryRemovalResult::default())
    }
}

#[post("/api/library/remove-track", auth: AuthSession)]
pub async fn remove_library_track(track_id: String) -> Result<LibraryRemovalResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let track = LibraryTrackRow::get_for_user(&track_id, &auth.0.sub)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Track not found"))?;
        let folder = Folder::get_by_id(&track.folder_id)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Library folder not found"))?;

        let result = remove_paths(
            &folder.path,
            &folder.path,
            std::slice::from_ref(&track.path),
            false,
        )
        .await;
        LibraryTrackRow::delete(&track_id)
            .await
            .map_err(server_error)?;
        LibraryReleaseRow::recount(&track.release_id)
            .await
            .map_err(server_error)?;

        crate::server_fns::media_server::trigger_media_scan(
            &auth.0.sub,
            std::path::Path::new(&folder.path),
        )
        .await;

        Ok(result)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = track_id;
        Ok(LibraryRemovalResult::default())
    }
}

#[post("/api/library/matches", auth: AuthSession)]
pub async fn check_library_matches(
    query: LibraryMatchQuery,
) -> Result<LibraryMatchResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        LibraryReleaseRow::match_query(&auth.0.sub, &query)
            .await
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = query;
        Ok(LibraryMatchResult::default())
    }
}

#[get("/api/library/scan-settings", _: AuthSession)]
pub async fn get_library_scan_settings() -> Result<LibraryScanSettings, ServerFnError> {
    #[cfg(feature = "server")]
    {
        Ok(crate::library::scan_settings().await)
    }
    #[cfg(not(feature = "server"))]
    Ok(LibraryScanSettings::default())
}

#[post("/api/library/ignore", auth: AuthSession)]
pub async fn set_library_release_ignored(
    release_id: String,
    ignored: bool,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        LibraryReleaseRow::get_for_user(&release_id, &auth.0.sub)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Release not found"))?;
        LibraryReleaseRow::set_ignored(&release_id, ignored)
            .await
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (release_id, ignored);
        Ok(())
    }
}

#[get("/api/library/download-activity", auth: AuthSession)]
pub async fn get_download_activity() -> Result<DownloadActivity, ServerFnError> {
    #[cfg(feature = "server")]
    {
        crate::models::download_request::DownloadRequestRow::activity(&auth.0.sub)
            .await
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    Ok(DownloadActivity::default())
}

#[post("/api/library/scan-settings", _: AuthSession)]
pub async fn update_library_scan_settings(
    settings: LibraryScanSettings,
) -> Result<LibraryScanSettings, ServerFnError> {
    #[cfg(feature = "server")]
    {
        crate::library::save_scan_settings(&settings)
            .await
            .map_err(server_error)?;
        Ok(crate::library::scan_settings().await)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = settings;
        Ok(LibraryScanSettings::default())
    }
}

#[cfg(feature = "server")]
async fn remove_paths(
    library_root: &str,
    release_path: &str,
    paths: &[String],
    remove_auxiliary: bool,
) -> LibraryRemovalResult {
    let root = std::path::Path::new(library_root);
    let mut result = LibraryRemovalResult::default();
    let mut removed = Vec::new();

    let beets_paths: Vec<String> = paths
        .iter()
        .filter(|path| std::path::Path::new(path).starts_with(root))
        .cloned()
        .collect();
    if !beets_paths.is_empty() {
        result.removed_from_beets = soulbeet::library::remove_from_beets(root, &beets_paths).await;
    }

    for path in paths {
        let file = std::path::Path::new(path);
        if !file.starts_with(root) {
            result.failed_files.push(path.clone());
            continue;
        }
        if !file.exists() {
            removed.push(path.clone());
            continue;
        }
        match tokio::fs::remove_file(file).await {
            Ok(()) => {
                result.removed_files += 1;
                removed.push(path.clone());
            }
            Err(e) => {
                warn!("Failed to remove {}: {}", path, e);
                result.failed_files.push(path.clone());
            }
        }
    }

    let release = std::path::Path::new(release_path);
    if remove_auxiliary && release != root && release.starts_with(root) && release.is_dir() {
        if let Ok(mut entries) = tokio::fs::read_dir(release).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let is_audio = soulbeet::library::is_audio_file(&path);
                let is_db = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with(".beets_library"))
                    .unwrap_or(false);
                if !is_audio && !is_db {
                    if let Ok(file_type) = entry.file_type().await {
                        if file_type.is_file() {
                            let _ = tokio::fs::remove_file(&path).await;
                        }
                    }
                }
            }
        }
        let _ = cleanup_empty_ancestors(release, root).await;
    }

    for path in &removed {
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = cleanup_empty_ancestors(parent, root).await;
        }
    }

    result
}
