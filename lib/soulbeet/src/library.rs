use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tracing::{info, warn};

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "wma", "aiff", "aif", "ape", "alac", "mpc",
    "wv",
];

const BEETS_UPDATE_TIMEOUT_SECS: u64 = 180;
const BEETS_QUERY_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Default)]
pub struct ScannedTrack {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration: Option<u32>,
    pub mb_track_id: Option<String>,
    pub mb_album_id: Option<String>,
    pub size: Option<u64>,
    pub has_metadata: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ScannedRelease {
    pub album: String,
    pub album_artist: String,
    pub year: Option<i32>,
    pub mb_album_id: Option<String>,
    pub path: String,
    pub tracks: Vec<ScannedTrack>,
}

#[derive(Debug, Clone, Default)]
pub struct ScannedLibrary {
    pub releases: Vec<ScannedRelease>,
    pub warnings: Vec<String>,
}

impl ScannedLibrary {
    pub fn track_count(&self) -> usize {
        self.releases.iter().map(|r| r.tracks.len()).sum()
    }
}

#[derive(Debug, Clone, Default)]
struct BeetsTrack {
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    duration: Option<u32>,
    mb_track_id: Option<String>,
    mb_album_id: Option<String>,
    year: Option<i32>,
}

fn beets_config_path() -> String {
    std::env::var("BEETS_CONFIG").unwrap_or_else(|_| "beets_config.yaml".to_string())
}

fn library_db(root: &Path) -> PathBuf {
    root.join(".beets_library.db")
}

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn audio_files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_audio_files(root, &mut files);
    files.sort();
    files
}

fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        "lost+found" | "@eaDir" | "#recycle" | ".Trash" | ".Trashes" | "node_modules"
    )
}

fn collect_audio_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || is_ignored_dir(name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_audio_files(&path, out);
        } else if file_type.is_file() && is_audio_file(&path) {
            out.push(path);
        }
    }
}

fn canonical_path(root: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw.trim());
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn clean_field(value: &str) -> String {
    value.trim().to_string()
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_number<T: std::str::FromStr>(value: &str) -> Option<T> {
    value.trim().parse().ok()
}

fn parse_duration(value: &str) -> Option<u32> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .map(|seconds| seconds.round() as u32)
}

fn parse_beets_line(root: &Path, line: &str) -> Option<(PathBuf, BeetsTrack)> {
    let parts: Vec<&str> = line.split('\u{1f}').collect();
    if parts.len() < 11 {
        return None;
    }
    let path = canonical_path(root, parts[0]);
    let track = BeetsTrack {
        title: clean_field(parts[1]),
        artist: clean_field(parts[2]),
        album: clean_field(parts[3]),
        album_artist: clean_field(parts[4]),
        track_number: parse_number(parts[5]),
        disc_number: parse_number(parts[6]),
        mb_track_id: non_empty(parts[7]),
        mb_album_id: non_empty(parts[8]),
        duration: parse_duration(parts[9]),
        year: parse_number(parts[10]),
    };
    Some((path, track))
}

async fn run_beet_update(root: &Path) -> Result<(), String> {
    let db = library_db(root);
    if !db.exists() {
        return Ok(());
    }
    let config = beets_config_path();
    let mut cmd = Command::new("beet");
    cmd.arg("-c")
        .arg(&config)
        .arg("-d")
        .arg(root)
        .arg("-l")
        .arg(&db)
        .arg("update")
        .arg("-M");
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let output = tokio::time::timeout(
        Duration::from_secs(BEETS_UPDATE_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "beet update timed out".to_string())?
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("beet update failed: {}", stderr.trim()));
    }
    Ok(())
}

async fn query_beets(root: &Path) -> Result<HashMap<PathBuf, BeetsTrack>, String> {
    let db = library_db(root);
    if !db.exists() {
        return Ok(HashMap::new());
    }
    let config = beets_config_path();
    let format = "$path\u{1f}$title\u{1f}$artist\u{1f}$album\u{1f}$albumartist\u{1f}$track\u{1f}$disc\u{1f}$mb_trackid\u{1f}$mb_albumid\u{1f}$length\u{1f}$year";

    let mut cmd = Command::new("beet");
    cmd.arg("-c")
        .arg(&config)
        .arg("-d")
        .arg(root)
        .arg("-l")
        .arg(&db)
        .arg("ls")
        .arg("-f")
        .arg(format)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let output = tokio::time::timeout(
        Duration::from_secs(BEETS_QUERY_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "beet ls timed out".to_string())?
    .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("no items") || stderr.trim().is_empty() {
            return Ok(HashMap::new());
        }
        return Err(format!("beet ls failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut map = HashMap::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        if let Some((path, track)) = parse_beets_line(root, line) {
            map.insert(path, track);
        }
    }
    Ok(map)
}

pub fn guess_file_title(path: &Path) -> (Option<u32>, String) {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let mut number = None;
    let mut title = stem.clone();
    if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_digit()) {
        return (stem.parse().ok(), String::new());
    }
    for separator in [" - ", " – ", " –", ". ", "_", " "] {
        if let Some((head, tail)) = stem.split_once(separator) {
            let head_trimmed = head.trim().trim_start_matches('0');
            if !head_trimmed.is_empty() && head_trimmed.chars().all(|c| c.is_ascii_digit()) {
                number = head.trim().parse().ok();
                title = tail.trim().to_string();
                break;
            }
        }
    }
    (number, title)
}

fn dir_year(name: &str) -> (String, Option<i32>) {
    for open in ['(', '['] {
        let close = if open == '(' { ')' } else { ']' };
        if let Some(start) = name.rfind(open) {
            if let Some(end) = name[start..].find(close) {
                let inner = &name[start + 1..start + end];
                if let Ok(year) = inner.trim().parse::<i32>() {
                    if (1900..=2100).contains(&year) {
                        let album = name[..start].trim().to_string();
                        return (album, Some(year));
                    }
                }
            }
        }
    }
    (name.trim().to_string(), None)
}

fn common_parent(paths: &[PathBuf]) -> PathBuf {
    let Some(first) = paths.first() else {
        return PathBuf::new();
    };
    let mut common = first.parent().unwrap_or(first).to_path_buf();
    for path in paths.iter().skip(1) {
        let Some(parent) = path.parent() else {
            continue;
        };
        while !parent.starts_with(&common) && common.parent().is_some() {
            common = common.parent().unwrap().to_path_buf();
        }
    }
    common
}

fn resolve_release_path(root: &Path, group: &mut ScannedRelease) {
    let parents: Vec<PathBuf> = group
        .tracks
        .iter()
        .map(|track| {
            PathBuf::from(&track.path)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.to_path_buf())
        })
        .collect();
    let common = common_parent(&parents);
    group.path = if common.starts_with(root) && common != root {
        common.to_string_lossy().to_string()
    } else {
        parents
            .first()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| root.to_string_lossy().to_string())
    };
}

fn scoped_key(value: &str) -> String {
    let key = shared::library::normalize_key(value);
    if key.is_empty() {
        "\u{0}unknown".to_string()
    } else {
        key
    }
}

pub async fn scan_library(root: &Path, sync_beets: bool) -> Result<ScannedLibrary, String> {
    if !root.is_dir() {
        return Err(format!("Library folder does not exist: {}", root.display()));
    }

    let mut warnings = Vec::new();

    if sync_beets && library_db(root).exists() {
        match run_beet_update(root).await {
            Ok(()) => info!("Synchronised beets library at {}", root.display()),
            Err(e) => {
                warn!("{}", e);
                warnings.push(e);
            }
        }
    }

    let beets = match query_beets(root).await {
        Ok(map) => map,
        Err(e) => {
            warn!("Could not read beets library at {}: {}", root.display(), e);
            warnings.push(e);
            HashMap::new()
        }
    };

    let walk_root = root.to_path_buf();
    let files = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        collect_audio_files(&walk_root, &mut files);
        files.sort();
        files
    })
    .await
    .map_err(|e| format!("Library walk failed: {e}"))?;

    if sync_beets && !beets.is_empty() {
        let file_set: std::collections::HashSet<&Path> =
            files.iter().map(PathBuf::as_path).collect();
        let stale: Vec<String> = beets
            .keys()
            .filter(|path| path.starts_with(root) && !file_set.contains(path.as_path()))
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        if !stale.is_empty() {
            info!(
                "Removing {} missing file(s) from the beets library at {}",
                stale.len(),
                root.display()
            );
            remove_from_beets(root, &stale).await;
        }
    }

    let mut releases: Vec<ScannedRelease> = Vec::new();
    let mut untagged: Vec<ScannedTrack> = Vec::new();

    for file in files {
        let size = tokio::fs::metadata(&file).await.ok().map(|meta| meta.len());
        let path = file.to_string_lossy().to_string();
        if let Some(meta) = beets.get(&file) {
            let album = if meta.album.is_empty() {
                file.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("Unknown Album")
                    .to_string()
            } else {
                meta.album.clone()
            };
            let album_artist = if meta.album_artist.is_empty() {
                meta.artist.clone()
            } else {
                meta.album_artist.clone()
            };
            let has_metadata = meta
                .mb_track_id
                .as_deref()
                .is_some_and(|value| !value.is_empty())
                || meta
                    .mb_album_id
                    .as_deref()
                    .is_some_and(|value| !value.is_empty());

            let (_, fallback_title) = guess_file_title(&file);
            let track = ScannedTrack {
                path: path.clone(),
                title: if meta.title.is_empty() {
                    fallback_title
                } else {
                    meta.title.clone()
                },
                artist: meta.artist.clone(),
                album: album.clone(),
                album_artist: album_artist.clone(),
                track_number: meta.track_number,
                disc_number: meta.disc_number,
                duration: meta.duration,
                mb_track_id: meta.mb_track_id.clone(),
                mb_album_id: meta.mb_album_id.clone(),
                size,
                has_metadata,
            };

            let key = (scoped_key(&album_artist), scoped_key(&album));
            if let Some(release) = releases.iter_mut().find(|release| {
                (
                    scoped_key(&release.album_artist),
                    scoped_key(&release.album),
                ) == key
            }) {
                if release.mb_album_id.is_none() {
                    release.mb_album_id = meta.mb_album_id.clone();
                }
                if release.year.is_none() {
                    release.year = meta.year;
                }
                release.tracks.push(track);
            } else {
                releases.push(ScannedRelease {
                    album,
                    album_artist,
                    year: meta.year,
                    mb_album_id: meta.mb_album_id.clone(),
                    path: String::new(),
                    tracks: vec![track],
                });
            }
        } else {
            let parent = file.parent().unwrap_or(root);
            let (album, _year) = dir_year(
                parent
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Unknown Album"),
            );
            let album_artist = parent
                .parent()
                .filter(|p| p.starts_with(root))
                .and_then(|p| p.file_name())
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
                .unwrap_or_else(|| "Unknown Artist".to_string());
            let (number, title) = guess_file_title(&file);
            untagged.push(ScannedTrack {
                path,
                title,
                artist: album_artist.clone(),
                album: album.clone(),
                album_artist,
                track_number: number,
                disc_number: None,
                duration: None,
                mb_track_id: None,
                mb_album_id: None,
                size,
                has_metadata: false,
            });
        }
    }

    for track in untagged {
        let parent = PathBuf::from(&track.path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        if let Some(release) = releases.iter_mut().find(|release| {
            release
                .tracks
                .iter()
                .any(|existing| Path::new(&existing.path).parent() == Some(parent.as_path()))
        }) {
            release.tracks.push(track);
            continue;
        }
        let (album, year) = dir_year(
            parent
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Unknown Album"),
        );
        let album_artist = parent
            .parent()
            .filter(|p| p.starts_with(root))
            .and_then(|p| p.file_name())
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
            .unwrap_or_else(|| "Unknown Artist".to_string());
        release_for_untagged(&mut releases, &parent, album, album_artist, year, track);
    }

    for release in &mut releases {
        resolve_release_path(root, release);
        release.tracks.sort_by(|a, b| {
            a.disc_number
                .cmp(&b.disc_number)
                .then(a.track_number.cmp(&b.track_number))
                .then(a.title.cmp(&b.title))
        });
    }
    releases.sort_by(|a, b| {
        a.album_artist
            .to_lowercase()
            .cmp(&b.album_artist.to_lowercase())
            .then(a.album.to_lowercase().cmp(&b.album.to_lowercase()))
    });

    Ok(ScannedLibrary { releases, warnings })
}

fn release_for_untagged(
    releases: &mut Vec<ScannedRelease>,
    parent: &Path,
    album: String,
    album_artist: String,
    year: Option<i32>,
    track: ScannedTrack,
) {
    if let Some(release) = releases.iter_mut().find(|release| {
        Path::new(&release.path) == parent
            && release.album.eq_ignore_ascii_case(&album)
            && release.album_artist.eq_ignore_ascii_case(&album_artist)
    }) {
        release.tracks.push(track);
        return;
    }
    releases.push(ScannedRelease {
        album,
        album_artist,
        year,
        mb_album_id: None,
        path: parent.to_string_lossy().to_string(),
        tracks: vec![track],
    });
}

pub async fn beets_known_paths(root: &Path) -> Result<std::collections::HashSet<PathBuf>, String> {
    let db = library_db(root);
    if !db.exists() {
        return Ok(std::collections::HashSet::new());
    }
    let config = beets_config_path();
    let mut cmd = Command::new("beet");
    cmd.arg("-c")
        .arg(&config)
        .arg("-d")
        .arg(root)
        .arg("-l")
        .arg(&db)
        .arg("ls")
        .arg("-f")
        .arg("$path")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let output = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| "beet ls timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Ok(std::collections::HashSet::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut paths = std::collections::HashSet::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let path = canonical_path(root, line);
        let path = tokio::fs::canonicalize(&path).await.unwrap_or(path);
        paths.insert(path);
    }
    Ok(paths)
}

pub async fn import_as_is(root: &Path, path: &Path) -> Result<(), String> {
    let db = library_db(root);
    let config = beets_config_path();
    let mut cmd = Command::new("beet");
    cmd.arg("-c")
        .arg(&config)
        .arg("-d")
        .arg(root)
        .arg("-l")
        .arg(&db)
        .arg("import")
        .arg("-A")
        .arg("-q")
        .arg("-M")
        .arg("-C")
        .arg("-s")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let output = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .map_err(|_| "beet import timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub async fn modify_track(
    root: &Path,
    path: &Path,
    fields: &[(&str, String)],
) -> Result<(), String> {
    let query = path.to_string_lossy().to_string();
    match run_modify_query(root, &query, fields).await {
        Ok(()) => Ok(()),
        Err(error)
            if error.contains("No matching objects to modify")
                || error.contains("matched no item") =>
        {
            let Some(id) = beets_item_id(root, path).await else {
                return Err(error);
            };
            run_modify_query(root, &format!("id:{id}"), fields).await
        }
        Err(error) => Err(error),
    }
}

async fn run_modify_query(
    root: &Path,
    query: &str,
    fields: &[(&str, String)],
) -> Result<(), String> {
    let db = library_db(root);
    if !db.exists() {
        return Err("beets library is not initialised for this folder".to_string());
    }
    let config = beets_config_path();
    let mut cmd = Command::new("beet");
    cmd.arg("-c")
        .arg(&config)
        .arg("-d")
        .arg(root)
        .arg("-l")
        .arg(&db)
        .arg("modify")
        .arg("-y")
        .arg("-M")
        .arg("-w")
        .arg(query);
    for (key, value) in fields {
        if value.is_empty() {
            continue;
        }
        cmd.arg(format!("{key}={value}"));
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let output = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| "beet modify timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stdout.contains("No items") || stderr.contains("No items") {
            Err("beets matched no item for this file".to_string())
        } else {
            Ok(())
        }
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

async fn beets_item_id(root: &Path, path: &Path) -> Option<String> {
    let db = library_db(root);
    if !db.exists() {
        return None;
    }
    let config = beets_config_path();
    let mut cmd = Command::new("beet");
    cmd.arg("-c")
        .arg(&config)
        .arg("-d")
        .arg(root)
        .arg("-l")
        .arg(&db)
        .arg("ls")
        .arg("-f")
        .arg("$id\u{1f}$path")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().ok()?;
    let output = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let canonical = tokio::fs::canonicalize(path).await.ok();
    for line in stdout.lines() {
        let Some((id, stored)) = line.split_once('\u{1f}') else {
            continue;
        };
        let stored = PathBuf::from(stored);
        let stored = if stored.is_absolute() {
            stored
        } else {
            root.join(stored)
        };
        let mut matches = stored == path;
        if !matches {
            if let Some(canonical) = canonical.as_ref() {
                matches = tokio::fs::canonicalize(&stored).await.ok().as_ref() == Some(canonical);
            }
        }
        if matches {
            return Some(id.to_string());
        }
    }
    None
}

pub async fn remove_from_beets(root: &Path, paths: &[String]) -> bool {
    let db = library_db(root);
    if !db.exists() || paths.is_empty() {
        return false;
    }
    let config = beets_config_path();
    let mut removed_any = false;

    for chunk in paths.chunks(50) {
        let mut cmd = Command::new("beet");
        cmd.arg("-c")
            .arg(&config)
            .arg("-d")
            .arg(root)
            .arg("-l")
            .arg(&db)
            .arg("remove")
            .arg("-f");
        for path in chunk {
            cmd.arg(path);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let Ok(child) = cmd.spawn() else {
            continue;
        };
        match tokio::time::timeout(Duration::from_secs(60), child.wait_with_output()).await {
            Ok(Ok(output)) => {
                if output.status.success() {
                    removed_any = true;
                } else {
                    warn!(
                        "beet remove failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
            }
            Ok(Err(e)) => warn!("beet remove error: {}", e),
            Err(_) => warn!("beet remove timed out"),
        }
    }
    removed_any
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_a_track_number_and_title_from_a_file_name() {
        let (number, title) = guess_file_title(Path::new("03 - A Song.flac"));
        assert_eq!(number, Some(3));
        assert_eq!(title, "A Song");
    }

    #[test]
    fn extracts_a_year_from_an_album_directory() {
        let (album, year) = dir_year("The Album (2019)");
        assert_eq!(album, "The Album");
        assert_eq!(year, Some(2019));
    }

    #[test]
    fn parses_beets_metadata_including_musicbrainz_ids() {
        let root = Path::new("/music/user");
        let line = "Artist/Album/01 - One.flac\u{1f}One\u{1f}Artist\u{1f}Album\u{1f}Album Artist\u{1f}1\u{1f}2\u{1f}recording-id\u{1f}release-id\u{1f}215.4\u{1f}2019";
        let (path, track) = parse_beets_line(root, line).unwrap();
        assert_eq!(path, Path::new("/music/user/Artist/Album/01 - One.flac"));
        assert_eq!(track.title, "One");
        assert_eq!(track.track_number, Some(1));
        assert_eq!(track.disc_number, Some(2));
        assert_eq!(track.mb_track_id.as_deref(), Some("recording-id"));
        assert_eq!(track.mb_album_id.as_deref(), Some("release-id"));
        assert_eq!(track.duration, Some(215));
        assert_eq!(track.year, Some(2019));
    }

    #[test]
    fn scan_groups_files_without_a_beets_database_by_directory() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Artist").join("Album (2020)");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join("01 - One.mp3"), b"one").unwrap();
        fs::write(album.join("02 - Two.mp3"), b"two").unwrap();

        let scan = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(scan_library(dir.path(), false))
            .unwrap();

        assert_eq!(scan.releases.len(), 1);
        let release = &scan.releases[0];
        assert_eq!(release.album, "Album");
        assert_eq!(release.album_artist, "Artist");
        assert_eq!(release.year, Some(2020));
        assert_eq!(release.tracks.len(), 2);
        assert!(release.tracks.iter().all(|track| !track.has_metadata));
        assert_eq!(release.tracks[0].track_number, Some(1));
    }
}
