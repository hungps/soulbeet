use std::collections::HashSet;
use std::path::Path as FsPath;

use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

use crate::models::app_config::AppConfig;
use crate::models::download_request::{DownloadRequestRow, DownloadRequestTrackRow};
use crate::models::folder::Folder;
use crate::models::library::LibraryTrackRow;
use crate::server_fns::download::utils::resolve_in_destination;
use crate::AuthSession;

#[derive(Debug, Deserialize)]
pub struct RequestAudioQuery {
    pub request: String,
    pub item: String,
}

#[derive(Debug, PartialEq, Eq)]
enum ByteRange {
    Full,
    Partial { start: u64, end: u64 },
    Unsatisfiable,
}

pub async fn stream_library_audio(
    auth: AuthSession,
    Path(track_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let track = match LibraryTrackRow::get_for_user(&track_id, &auth.0.sub).await {
        Ok(Some(track)) => track,
        Ok(None) => return not_found(),
        Err(error) => {
            tracing::warn!("Failed to load library track for playback: {error}");
            return internal_error();
        }
    };
    let folder = match Folder::get_by_id(&track.folder_id).await {
        Ok(Some(folder)) if folder.user_id == auth.0.sub => folder,
        Ok(_) => return not_found(),
        Err(error) => {
            tracing::warn!("Failed to load library folder for playback: {error}");
            return internal_error();
        }
    };

    serve_file(&track.path, FsPath::new(&folder.path), &headers).await
}

pub async fn stream_request_audio(
    auth: AuthSession,
    Query(query): Query<RequestAudioQuery>,
    headers: HeaderMap,
) -> Response {
    let track =
        match DownloadRequestTrackRow::find_for_request(&query.request, &auth.0.sub, &query.item)
            .await
        {
            Ok(Some(track)) => track,
            // A request can join a batch another request started; the row then
            // carries the batch's id, not its own request's.
            Ok(None) => match DownloadRequestTrackRow::find_by_item(&auth.0.sub, &query.item).await {
                Ok(Some(track)) => track,
                Ok(None) => {
                    tracing::warn!(
                        "No download row for playback: request {} item {}",
                        query.request,
                        query.item
                    );
                    return not_found();
                }
                Err(error) => {
                    tracing::warn!("Failed to load download track for playback: {error}");
                    return internal_error();
                }
            },
            Err(error) => {
                tracing::warn!("Failed to load download track for playback: {error}");
                return internal_error();
            }
        };
    let root = AppConfig::download_path().await;
    let Some(path) = request_file_path(&track, &root, &auth.0.sub).await else {
        tracing::warn!(
            "Could not find the downloaded file for playback: {} under {}",
            track.item_key,
            root.display()
        );
        return not_found();
    };

    serve_file(&path.to_string_lossy(), &root, &headers).await
}

async fn request_file_path(
    track: &DownloadRequestTrackRow,
    root: &FsPath,
    user_id: &str,
) -> Option<std::path::PathBuf> {
    if let Some(local_path) = track.local_path.as_deref() {
        let kept = std::path::PathBuf::from(local_path);
        if tokio::fs::metadata(&kept)
            .await
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return Some(kept);
        }
        // A kept row points at the folder its files were left in.
        if let (Some(parent), Some(name)) = (
            kept.parent().and_then(FsPath::to_str),
            kept.file_name().and_then(|name| name.to_str()),
        ) {
            if let Some(path) =
                resolve_in_destination(&track.item_key, FsPath::new(parent), name, &HashSet::new())
            {
                return Some(path);
            }
        }
    }
    if let Ok(Some(request)) = DownloadRequestRow::get_for_user(&track.request_id, user_id).await {
        let destination = shared::download::batch_folder(&request.label, &request.id);
        if let Some(path) =
            resolve_in_destination(&track.item_key, root, &destination, &HashSet::new())
        {
            return Some(path);
        }
    }
    let slskd_root = AppConfig::slskd_download_path().await;
    crate::server_fns::download::utils::resolve_download_path_mapped(
        &track.item_key,
        root,
        slskd_root.as_deref(),
    )
    .map(std::path::PathBuf::from)
}

async fn serve_file(path: &str, root: &FsPath, headers: &HeaderMap) -> Response {
    let root = match tokio::fs::canonicalize(root).await {
        Ok(root) => root,
        Err(error) => {
            tracing::warn!(
                "Could not resolve the audio root {}: {error}",
                root.display()
            );
            return not_found();
        }
    };
    let path = match tokio::fs::canonicalize(path).await {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!("Could not resolve the audio file {path}: {error}");
            return not_found();
        }
    };
    if !path.starts_with(&root) {
        tracing::warn!(
            "Refusing to stream {}: it is outside {}",
            path.display(),
            root.display()
        );
        return forbidden();
    }

    let mut file = match File::open(&path).await {
        Ok(file) => file,
        Err(_) => return not_found(),
    };
    let length = match file.metadata().await {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        _ => return not_found(),
    };

    match parse_byte_range(headers, length) {
        ByteRange::Unsatisfiable => range_not_satisfiable(length),
        ByteRange::Full => stream_file(file, length, length, None, mime_type(&path)),
        ByteRange::Partial { start, end } => {
            if file.seek(SeekFrom::Start(start)).await.is_err() {
                return internal_error();
            }
            let partial_length = end - start + 1;
            stream_file(
                file,
                partial_length,
                length,
                Some((start, end)),
                mime_type(&path),
            )
        }
    }
}

fn stream_file(
    file: File,
    content_length: u64,
    total_length: u64,
    range: Option<(u64, u64)>,
    content_type: &'static str,
) -> Response {
    let body = Body::from_stream(ReaderStream::new(file.take(content_length)));
    let mut response = Response::new(body);
    let status = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    *response.status_mut() = status;

    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=0, must-revalidate"),
    );
    if let Some((start, end)) = range {
        if let Ok(value) = HeaderValue::from_str(&format!("bytes {start}-{end}/{total_length}")) {
            headers.insert(header::CONTENT_RANGE, value);
        }
    }

    response
}

fn parse_byte_range(headers: &HeaderMap, length: u64) -> ByteRange {
    let Some(value) = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
    else {
        return ByteRange::Full;
    };
    let Some(spec) = value.strip_prefix("bytes=") else {
        return ByteRange::Full;
    };
    let Some(spec) = spec.split(',').next() else {
        return ByteRange::Unsatisfiable;
    };
    let Some((start, end)) = spec.trim().split_once('-') else {
        return ByteRange::Unsatisfiable;
    };

    if length == 0 {
        return ByteRange::Unsatisfiable;
    }

    match (start.trim(), end.trim()) {
        ("", "") => ByteRange::Unsatisfiable,
        ("", suffix) => {
            let Ok(suffix) = suffix.parse::<u64>() else {
                return ByteRange::Unsatisfiable;
            };
            if suffix == 0 {
                return ByteRange::Unsatisfiable;
            }
            ByteRange::Partial {
                start: length.saturating_sub(suffix),
                end: length - 1,
            }
        }
        (start, "") => {
            let Ok(start) = start.parse::<u64>() else {
                return ByteRange::Unsatisfiable;
            };
            if start >= length {
                return ByteRange::Unsatisfiable;
            }
            ByteRange::Partial {
                start,
                end: length - 1,
            }
        }
        (start, end) => {
            let (Ok(start), Ok(end)) = (start.parse::<u64>(), end.parse::<u64>()) else {
                return ByteRange::Unsatisfiable;
            };
            if start > end || start >= length {
                return ByteRange::Unsatisfiable;
            }
            ByteRange::Partial {
                start,
                end: end.min(length - 1),
            }
        }
    }
}

fn mime_type(path: &FsPath) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "m4b" | "mp4" | "alac" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        "wma" => "audio/x-ms-wma",
        "ape" => "audio/x-ape",
        "wv" => "audio/x-wavpack",
        "mpc" => "audio/x-musepack",
        "tta" => "audio/x-tta",
        "dsf" => "audio/dsf",
        "dff" => "audio/x-dff",
        _ => "application/octet-stream",
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Audio file not found").into_response()
}

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "Audio file is outside its root").into_response()
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Could not read audio file",
    )
        .into_response()
}

fn range_not_satisfiable(length: u64) -> Response {
    let mut response = (StatusCode::RANGE_NOT_SATISFIABLE, "Invalid range").into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{length}")) {
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn range(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn a_missing_range_streams_the_whole_file() {
        assert_eq!(parse_byte_range(&HeaderMap::new(), 100), ByteRange::Full);
    }

    #[test]
    fn open_ranges_end_at_the_file_end() {
        assert_eq!(
            parse_byte_range(&range("bytes=10-"), 100),
            ByteRange::Partial { start: 10, end: 99 }
        );
    }

    #[test]
    fn closed_ranges_are_clamped() {
        assert_eq!(
            parse_byte_range(&range("bytes=10-20"), 100),
            ByteRange::Partial { start: 10, end: 20 }
        );
        assert_eq!(
            parse_byte_range(&range("bytes=90-120"), 100),
            ByteRange::Partial { start: 90, end: 99 }
        );
    }

    #[test]
    fn suffix_ranges_read_from_the_end() {
        assert_eq!(
            parse_byte_range(&range("bytes=-20"), 100),
            ByteRange::Partial { start: 80, end: 99 }
        );
    }

    #[test]
    fn malformed_unsatisfiable_ranges_are_refused() {
        assert_eq!(
            parse_byte_range(&range("bytes=100-"), 100),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            parse_byte_range(&range("bytes=20-10"), 100),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            parse_byte_range(&range("bytes=-0"), 100),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            parse_byte_range(&range("bytes="), 100),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            parse_byte_range(&range("bytes=0-"), 0),
            ByteRange::Unsatisfiable
        );
    }

    #[test]
    fn common_audio_extensions_get_browser_friendly_types() {
        assert_eq!(mime_type(FsPath::new("song.FLAC")), "audio/flac");
        assert_eq!(mime_type(FsPath::new("song.mp3")), "audio/mpeg");
        assert_eq!(mime_type(FsPath::new("song.m4a")), "audio/mp4");
        assert_eq!(
            mime_type(FsPath::new("song.unknown")),
            "application/octet-stream"
        );
    }
}
