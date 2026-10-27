use std::collections::{HashMap, HashSet};

use shared::library::{
    normalize_key, LibraryFolderInfo, LibraryMatchInfo, LibraryMatchQuery, LibraryMatchResult,
    LibraryRelease, LibraryReleaseDetail, LibraryTrackInfo, ReleaseCompleteness,
};
use soulbeet::library::{ScannedLibrary, ScannedTrack};
use uuid::Uuid;

use crate::db::DB;

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct LibraryReleaseRow {
    pub id: String,
    pub folder_id: String,
    pub folder_path: String,
    pub album: String,
    pub album_artist: String,
    /// The first artist MusicBrainz credits for the release, when it resolved
    /// one. The scan cannot know it, so it is kept across rescans.
    pub mb_album_artist: Option<String>,
    pub album_key: String,
    pub album_artist_key: String,
    pub album_disambig: Option<String>,
    pub media: Option<String>,
    pub country: Option<String>,
    pub year: Option<i64>,
    pub mb_album_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    pub release_path: String,
    pub track_count: i64,
    pub matched_track_count: i64,
    pub expected_track_count: Option<i64>,
    pub total_size: i64,
    pub audio_format: Option<String>,
    pub bitrate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub sample_rate: Option<i64>,
    pub quality_varies: i64,
    pub scanned_at: String,
    pub ignored: i64,
    pub folder_name: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct LibraryTrackRow {
    pub id: String,
    pub release_id: String,
    pub folder_id: String,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub title_key: String,
    pub artist_key: String,
    pub album_key: String,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub duration: Option<i64>,
    pub mb_track_id: Option<String>,
    pub mb_album_id: Option<String>,
    pub size: Option<i64>,
    pub audio_format: Option<String>,
    pub bitrate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub sample_rate: Option<i64>,
    pub matched: i64,
    pub indexed_at: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct LibraryScanStateRow {
    pub folder_id: String,
    pub status: String,
    pub last_scanned_at: Option<String>,
    pub error: Option<String>,
    pub release_count: i64,
    pub track_count: i64,
}

fn completeness(track_count: i64, matched: i64, expected: Option<i64>) -> ReleaseCompleteness {
    if track_count <= 0 || matched <= 0 {
        return ReleaseCompleteness::Unmatched;
    }
    let all_matched = matched >= track_count;
    let count_ok = expected.map(|value| value <= track_count).unwrap_or(true);
    if all_matched && count_ok {
        ReleaseCompleteness::Complete
    } else {
        ReleaseCompleteness::Partial
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct AudioQuality {
    audio_format: Option<String>,
    bitrate: Option<u32>,
    bit_depth: Option<u32>,
    sample_rate: Option<u32>,
}

impl AudioQuality {
    fn of(track: &ScannedTrack) -> Self {
        Self {
            audio_format: track.audio_format.clone(),
            bitrate: track.bitrate,
            bit_depth: track.bit_depth,
            sample_rate: track.sample_rate,
        }
    }
}

fn release_quality(tracks: &[ScannedTrack]) -> (AudioQuality, bool) {
    let mut counts: HashMap<AudioQuality, usize> = HashMap::new();
    let mut formats = HashSet::new();
    for track in tracks {
        let quality = AudioQuality::of(track);
        if quality.audio_format.is_none() {
            continue;
        }
        formats.extend(quality.audio_format.clone());
        *counts.entry(quality).or_default() += 1;
    }
    let quality = counts
        .into_iter()
        .max_by(|(left, left_count), (right, right_count)| {
            left_count.cmp(right_count).then_with(|| left.cmp(right))
        })
        .map(|(quality, _)| quality)
        .unwrap_or_default();
    (quality, formats.len() > 1)
}

fn audio_format_from_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_lowercase())
        .filter(|extension| !extension.is_empty())
}

impl LibraryReleaseRow {
    pub fn to_shared(&self) -> LibraryRelease {
        LibraryRelease {
            id: self.id.clone(),
            folder_id: self.folder_id.clone(),
            folder_name: self.folder_name.clone(),
            folder_path: self.folder_path.clone(),
            album: self.album.clone(),
            album_artist: self.album_artist.clone(),
            main_artist: self
                .mb_album_artist
                .clone()
                .filter(|value| !value.trim().is_empty()),
            album_disambig: self.album_disambig.clone(),
            media: self.media.clone(),
            country: self.country.clone(),
            year: self.year.map(|value| value as i32),
            mbid: self.mb_album_id.clone().filter(|value| !value.is_empty()),
            mb_release_group_id: self
                .mb_release_group_id
                .clone()
                .filter(|value| !value.is_empty()),
            path: self.release_path.clone(),
            track_count: self.track_count.max(0) as u32,
            matched_track_count: self.matched_track_count.max(0) as u32,
            expected_track_count: self.expected_track_count.map(|value| value.max(0) as u32),
            total_size: self.total_size.max(0) as u64,
            audio_format: self.audio_format.clone(),
            bitrate: self.bitrate.map(|value| value.max(0) as u32),
            bit_depth: self.bit_depth.map(|value| value.max(0) as u32),
            sample_rate: self.sample_rate.map(|value| value.max(0) as u32),
            quality_varies: self.quality_varies != 0,
            completeness: completeness(
                self.track_count,
                self.matched_track_count,
                self.expected_track_count,
            ),
            scanned_at: self.scanned_at.clone(),
            ignored: self.ignored != 0,
        }
    }
}

impl LibraryTrackRow {
    pub fn to_shared(&self) -> LibraryTrackInfo {
        LibraryTrackInfo {
            id: self.id.clone(),
            release_id: self.release_id.clone(),
            path: self.path.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            album_artist: self.album_artist.clone(),
            track_number: self.track_number.map(|value| value.max(0) as u32),
            disc_number: self.disc_number.map(|value| value.max(0) as u32),
            duration: self.duration.map(|value| value.max(0) as u32),
            mb_track_id: self.mb_track_id.clone().filter(|value| !value.is_empty()),
            mb_album_id: self.mb_album_id.clone().filter(|value| !value.is_empty()),
            size: self.size.map(|value| value.max(0) as u64),
            audio_format: self
                .audio_format
                .clone()
                .or_else(|| audio_format_from_path(&self.path)),
            bitrate: self.bitrate.map(|value| value.max(0) as u32),
            bit_depth: self.bit_depth.map(|value| value.max(0) as u32),
            sample_rate: self.sample_rate.map(|value| value.max(0) as u32),
            matched: self.matched != 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PriorRelease {
    ignored: i64,
    expected: Option<i64>,
    main_artist: Option<String>,
}

impl LibraryReleaseRow {
    pub async fn replace_folder(
        folder_id: &str,
        folder_path: &str,
        scan: &ScannedLibrary,
    ) -> Result<(usize, usize), String> {
        let prior: std::collections::HashMap<String, PriorRelease> =
            sqlx::query_as::<_, (String, i64, Option<i64>, Option<String>)>(
                "SELECT release_path, ignored, expected_track_count, mb_album_artist
                 FROM library_releases WHERE folder_id = ?",
            )
            .bind(folder_id)
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(path, ignored, expected, main_artist)| {
                (
                    path,
                    PriorRelease {
                        ignored,
                        expected,
                        main_artist,
                    },
                )
            })
            .collect();

        let mut tx = DB.begin().await.map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM library_tracks WHERE folder_id = ?")
            .bind(folder_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM library_releases WHERE folder_id = ?")
            .bind(folder_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        let scanned_at = chrono::Utc::now().to_rfc3339();
        let mut track_total = 0usize;

        for release in &scan.releases {
            if release.tracks.is_empty() {
                continue;
            }
            let release_id = Uuid::new_v4().to_string();
            let matched = release
                .tracks
                .iter()
                .filter(|track| track.has_metadata)
                .count() as i64;
            let total_size: i64 = release
                .tracks
                .iter()
                .filter_map(|track| track.size)
                .map(|size| size as i64)
                .sum();
            let release_path = if release.path.is_empty() {
                folder_path.to_string()
            } else {
                release.path.clone()
            };
            let (quality, quality_varies) = release_quality(&release.tracks);

            sqlx::query(
                "INSERT INTO library_releases
                    (id, folder_id, folder_path, album, album_artist, album_key, album_artist_key,
                     album_disambig, media, country, year, mb_album_id, mb_release_group_id,
                     release_path, track_count, matched_track_count, expected_track_count,
                     total_size, audio_format, bitrate, bit_depth, sample_rate, quality_varies,
                     scanned_at, ignored, mb_album_artist)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&release_id)
            .bind(folder_id)
            .bind(folder_path)
            .bind(&release.album)
            .bind(&release.album_artist)
            .bind(normalize_key(&release.album))
            .bind(normalize_key(&release.album_artist))
            .bind(&release.album_disambig)
            .bind(&release.media)
            .bind(&release.country)
            .bind(release.year.map(|year| year as i64))
            .bind(&release.mb_album_id)
            .bind(&release.mb_release_group_id)
            .bind(&release_path)
            .bind(release.tracks.len() as i64)
            .bind(matched)
            .bind(
                release
                    .expected_track_count
                    .map(|value| value as i64)
                    .or_else(|| prior.get(&release_path).and_then(|prior| prior.expected)),
            )
            .bind(total_size)
            .bind(&quality.audio_format)
            .bind(quality.bitrate.map(|value| value as i64))
            .bind(quality.bit_depth.map(|value| value as i64))
            .bind(quality.sample_rate.map(|value| value as i64))
            .bind(if quality_varies { 1i64 } else { 0i64 })
            .bind(&scanned_at)
            .bind(prior.get(&release_path).map(|prior| prior.ignored).unwrap_or(0))
            .bind(
                prior
                    .get(&release_path)
                    .and_then(|prior| prior.main_artist.clone()),
            )
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

            for track in &release.tracks {
                sqlx::query(
                    "INSERT INTO library_tracks
                        (id, release_id, folder_id, path, title, artist, album, album_artist,
                         title_key, artist_key, album_key, track_number, disc_number, duration,
                         mb_track_id, mb_album_id, size, audio_format, bitrate, bit_depth,
                         sample_rate, matched, indexed_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(&release_id)
                .bind(folder_id)
                .bind(&track.path)
                .bind(&track.title)
                .bind(&track.artist)
                .bind(&track.album)
                .bind(&track.album_artist)
                .bind(normalize_key(&track.title))
                .bind(normalize_key(&track.artist))
                .bind(normalize_key(&track.album))
                .bind(track.track_number.map(|value| value as i64))
                .bind(track.disc_number.map(|value| value as i64))
                .bind(track.duration.map(|value| value as i64))
                .bind(&track.mb_track_id)
                .bind(&track.mb_album_id)
                .bind(track.size.map(|value| value as i64))
                .bind(&track.audio_format)
                .bind(track.bitrate.map(|value| value as i64))
                .bind(track.bit_depth.map(|value| value as i64))
                .bind(track.sample_rate.map(|value| value as i64))
                .bind(if track.has_metadata { 1i64 } else { 0i64 })
                .bind(&scanned_at)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
                track_total += 1;
            }
        }

        tx.commit().await.map_err(|e| e.to_string())?;
        Ok((scan.releases.len(), track_total))
    }

    pub async fn list(user_id: &str, folder_id: Option<&str>) -> Result<Vec<Self>, String> {
        let rows = if let Some(folder_id) = folder_id {
            sqlx::query_as::<_, Self>(
                "SELECT r.*, f.name AS folder_name
                 FROM library_releases r
                 JOIN folders f ON f.id = r.folder_id
                 WHERE f.user_id = ? AND r.folder_id = ?
                 ORDER BY r.album_artist COLLATE NOCASE, r.album COLLATE NOCASE",
            )
            .bind(user_id)
            .bind(folder_id)
            .fetch_all(&*DB)
            .await
        } else {
            sqlx::query_as::<_, Self>(
                "SELECT r.*, f.name AS folder_name
                 FROM library_releases r
                 JOIN folders f ON f.id = r.folder_id
                 WHERE f.user_id = ?
                 ORDER BY r.album_artist COLLATE NOCASE, r.album COLLATE NOCASE",
            )
            .bind(user_id)
            .fetch_all(&*DB)
            .await
        };
        rows.map_err(|e| e.to_string())
    }

    pub async fn get_for_user(id: &str, user_id: &str) -> Result<Option<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT r.*, f.name AS folder_name
             FROM library_releases r
             JOIN folders f ON f.id = r.folder_id
             WHERE r.id = ? AND f.user_id = ?",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn set_ignored(id: &str, ignored: bool) -> Result<(), String> {
        sqlx::query("UPDATE library_releases SET ignored = ? WHERE id = ?")
            .bind(if ignored { 1i64 } else { 0i64 })
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn set_expected_track_count(id: &str, expected: u32) -> Result<(), String> {
        sqlx::query("UPDATE library_releases SET expected_track_count = ? WHERE id = ?")
            .bind(expected as i64)
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn set_main_artist(id: &str, artist: &str) -> Result<(), String> {
        sqlx::query("UPDATE library_releases SET mb_album_artist = ? WHERE id = ?")
            .bind(artist)
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete(id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM library_tracks WHERE release_id = ?")
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM library_releases WHERE id = ?")
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn recount(id: &str) -> Result<Option<Self>, String> {
        let counts = sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT COUNT(*), COALESCE(SUM(matched), 0), COALESCE(SUM(size), 0)
             FROM library_tracks WHERE release_id = ?",
        )
        .bind(id)
        .fetch_one(&*DB)
        .await
        .map_err(|e| e.to_string())?;

        if counts.0 == 0 {
            Self::delete(id).await?;
            return Ok(None);
        }

        sqlx::query(
            "UPDATE library_releases
             SET track_count = ?, matched_track_count = ?, total_size = ?
             WHERE id = ?",
        )
        .bind(counts.0)
        .bind(counts.1)
        .bind(counts.2)
        .bind(id)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query_as::<_, Self>(
            "SELECT r.*, f.name AS folder_name
             FROM library_releases r
             JOIN folders f ON f.id = r.folder_id
             WHERE r.id = ?",
        )
        .bind(id)
        .fetch_optional(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn detail(id: &str, user_id: &str) -> Result<Option<LibraryReleaseDetail>, String> {
        let Some(release) = Self::get_for_user(id, user_id).await? else {
            return Ok(None);
        };
        let tracks = LibraryTrackRow::list_for_release(id).await?;
        Ok(Some(LibraryReleaseDetail {
            release: release.to_shared(),
            tracks: tracks.iter().map(LibraryTrackRow::to_shared).collect(),
            missing_tracks: Vec::new(),
        }))
    }

    pub async fn has_nested_releases(
        id: &str,
        folder_id: &str,
        release_path: &str,
    ) -> Result<bool, String> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM library_releases
             WHERE folder_id = ? AND id != ?
               AND substr(release_path, 1, length(?) + 1) = ? || '/'",
        )
        .bind(folder_id)
        .bind(id)
        .bind(release_path)
        .bind(release_path)
        .fetch_one(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(count > 0)
    }

    pub async fn folder_infos(user_id: &str) -> Result<Vec<LibraryFolderInfo>, String> {
        let folders = crate::models::folder::Folder::get_all_by_user(user_id).await?;
        let states = sqlx::query_as::<_, LibraryScanStateRow>(
            "SELECT s.* FROM library_scan_state s
             JOIN folders f ON f.id = s.folder_id
             WHERE f.user_id = ?",
        )
        .bind(user_id)
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        let state_map: std::collections::HashMap<String, LibraryScanStateRow> = states
            .into_iter()
            .map(|state| (state.folder_id.clone(), state))
            .collect();

        let mut infos = Vec::new();
        for folder in folders {
            let counts = sqlx::query_as::<_, (i64, i64)>(
                "SELECT
                    (SELECT COUNT(*) FROM library_releases WHERE folder_id = ?),
                    (SELECT COUNT(*) FROM library_tracks WHERE folder_id = ?)",
            )
            .bind(&folder.id)
            .bind(&folder.id)
            .fetch_one(&*DB)
            .await
            .map_err(|e| e.to_string())?;
            let state = state_map.get(&folder.id);
            infos.push(LibraryFolderInfo {
                id: folder.id,
                name: folder.name,
                path: folder.path,
                release_count: counts.0.max(0) as u32,
                track_count: counts.1.max(0) as u32,
                last_scanned_at: state.and_then(|state| state.last_scanned_at.clone()),
                scan_status: state
                    .map(|state| state.status.clone())
                    .unwrap_or_else(|| "idle".to_string()),
                scan_error: state.and_then(|state| state.error.clone()),
            });
        }
        Ok(infos)
    }

    pub async fn match_query(
        user_id: &str,
        query: &LibraryMatchQuery,
    ) -> Result<LibraryMatchResult, String> {
        let mut result = LibraryMatchResult::default();

        for album in &query.albums {
            let rows = sqlx::query_as::<_, LibraryReleaseRow>(
                "SELECT r.*, f.name AS folder_name
                 FROM library_releases r
                 JOIN folders f ON f.id = r.folder_id
                 WHERE f.user_id = ?
                   AND ((r.album_artist_key = ? AND r.album_key = ?)
                        OR (? IS NOT NULL AND r.mb_album_id = ?))
                 ORDER BY r.scanned_at DESC",
            )
            .bind(user_id)
            .bind(normalize_key(&album.artist))
            .bind(normalize_key(&album.album))
            .bind(album.mbid.as_deref())
            .bind(album.mbid.as_deref())
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())?;
            result
                .albums
                .insert(album.key.clone(), aggregate_matches(&rows));
        }

        for track in &query.tracks {
            let rows = sqlx::query_as::<_, LibraryReleaseRow>(
                "SELECT DISTINCT r.*, f.name AS folder_name
                 FROM library_tracks t
                 JOIN library_releases r ON r.id = t.release_id
                 JOIN folders f ON f.id = r.folder_id
                 WHERE f.user_id = ? AND t.artist_key = ? AND t.title_key = ?
                 ORDER BY r.scanned_at DESC",
            )
            .bind(user_id)
            .bind(normalize_key(&track.artist))
            .bind(normalize_key(&track.title))
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())?;
            let mut info = aggregate_matches(&rows);
            let track_ids = sqlx::query_scalar::<_, String>(
                "SELECT t.id
                 FROM library_tracks t
                 JOIN folders f ON f.id = t.folder_id
                 WHERE f.user_id = ? AND t.artist_key = ? AND t.title_key = ?
                 ORDER BY t.id",
            )
            .bind(user_id)
            .bind(normalize_key(&track.artist))
            .bind(normalize_key(&track.title))
            .fetch_all(&*DB)
            .await
            .map_err(|e| e.to_string())?;
            info.track_ids = track_ids;
            result.tracks.insert(track.key.clone(), info);
        }

        Ok(result)
    }
}

fn aggregate_matches(rows: &[LibraryReleaseRow]) -> LibraryMatchInfo {
    if rows.is_empty() {
        return LibraryMatchInfo::default();
    }
    let mut release_ids = Vec::new();
    let mut folder_ids = Vec::new();
    let mut folder_names = Vec::new();
    let mut track_count = 0u32;
    let mut matched_track_count = 0u32;
    let mut complete = false;
    for row in rows {
        release_ids.push(row.id.clone());
        if !folder_ids.contains(&row.folder_id) {
            folder_ids.push(row.folder_id.clone());
        }
        if !folder_names.contains(&row.folder_name) {
            folder_names.push(row.folder_name.clone());
        }
        track_count += row.track_count.max(0) as u32;
        matched_track_count += row.matched_track_count.max(0) as u32;
        complete |= row.to_shared().completeness == ReleaseCompleteness::Complete;
    }
    LibraryMatchInfo {
        matched: true,
        complete,
        release_ids,
        track_ids: Vec::new(),
        folder_ids,
        folder_names,
        track_count,
        matched_track_count,
    }
}

impl LibraryTrackRow {
    pub async fn list_for_release(release_id: &str) -> Result<Vec<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT * FROM library_tracks
             WHERE release_id = ?
             ORDER BY disc_number, track_number, title COLLATE NOCASE",
        )
        .bind(release_id)
        .fetch_all(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn get_for_user(id: &str, user_id: &str) -> Result<Option<Self>, String> {
        sqlx::query_as::<_, Self>(
            "SELECT t.* FROM library_tracks t
             JOIN folders f ON f.id = t.folder_id
             WHERE t.id = ? AND f.user_id = ?",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&*DB)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn delete(id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM library_tracks WHERE id = ?")
            .bind(id)
            .execute(&*DB)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl LibraryScanStateRow {
    pub async fn set_status(
        folder_id: &str,
        status: &str,
        error: Option<&str>,
        last_scanned_at: Option<&str>,
        release_count: i64,
        track_count: i64,
    ) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO library_scan_state
                (folder_id, status, last_scanned_at, error, release_count, track_count)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(folder_id) DO UPDATE SET
                status = excluded.status,
                last_scanned_at = COALESCE(excluded.last_scanned_at, library_scan_state.last_scanned_at),
                error = excluded.error,
                release_count = excluded.release_count,
                track_count = excluded.track_count",
        )
        .bind(folder_id)
        .bind(status)
        .bind(last_scanned_at)
        .bind(error)
        .bind(release_count)
        .bind(track_count)
        .execute(&*DB)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get(folder_id: &str) -> Result<Option<Self>, String> {
        sqlx::query_as::<_, Self>("SELECT * FROM library_scan_state WHERE folder_id = ?")
            .bind(folder_id)
            .fetch_optional(&*DB)
            .await
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completeness_needs_every_track_matched_and_the_expected_count_present() {
        assert_eq!(completeness(0, 0, None), ReleaseCompleteness::Unmatched);
        assert_eq!(completeness(3, 0, None), ReleaseCompleteness::Unmatched);
        assert_eq!(completeness(3, 2, None), ReleaseCompleteness::Partial);
        assert_eq!(completeness(3, 3, None), ReleaseCompleteness::Complete);
        assert_eq!(completeness(3, 3, Some(5)), ReleaseCompleteness::Partial);
        assert_eq!(completeness(5, 5, Some(5)), ReleaseCompleteness::Complete);
    }

    #[test]
    fn aggregate_prefers_a_complete_release_but_reports_all_editions() {
        let row = |id: &str, matched: i64, expected: Option<i64>| LibraryReleaseRow {
            id: id.to_string(),
            folder_id: "folder".to_string(),
            folder_path: "/music".to_string(),
            album: "Album".to_string(),
            album_artist: "Artist".to_string(),
            mb_album_artist: None,
            album_key: "album".to_string(),
            album_artist_key: "artist".to_string(),
            album_disambig: None,
            media: None,
            country: None,
            year: None,
            mb_album_id: None,
            mb_release_group_id: None,
            release_path: "/music/Artist/Album".to_string(),
            track_count: 2,
            matched_track_count: matched,
            expected_track_count: expected,
            total_size: 0,
            audio_format: None,
            bitrate: None,
            bit_depth: None,
            sample_rate: None,
            quality_varies: 0,
            scanned_at: String::new(),
            ignored: 0,
            folder_name: "Main".to_string(),
        };
        let info = aggregate_matches(&[row("a", 2, None), row("b", 1, None)]);
        assert!(info.matched);
        assert!(info.complete);
        assert_eq!(info.release_ids, vec!["a", "b"]);
        assert_eq!(info.track_count, 4);
    }

    #[test]
    fn release_quality_uses_the_most_common_track_quality() {
        let track = |format: &str, bitrate: Option<u32>, bit_depth: Option<u32>| ScannedTrack {
            audio_format: Some(format.to_string()),
            bitrate,
            bit_depth,
            sample_rate: Some(44100),
            ..ScannedTrack::default()
        };
        let (quality, varies) = release_quality(&[
            track("flac", None, Some(16)),
            track("flac", None, Some(16)),
            track("mp3", Some(320), None),
        ]);
        assert_eq!(quality.audio_format.as_deref(), Some("flac"));
        assert_eq!(quality.bit_depth, Some(16));
        assert!(varies);

        let (quality, varies) = release_quality(&[track("mp3", Some(320), None)]);
        assert_eq!(quality.audio_format.as_deref(), Some("mp3"));
        assert!(!varies);
    }
}
