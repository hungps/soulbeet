use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A track from a beets library
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryTrack {
    pub path: String,
    pub artist: String,
    pub title: String,
    pub album: String,
    pub album_artist: String,
    pub library_path: String,
}

/// A group of duplicate tracks (same artist + title across different libraries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub artist: String,
    pub title: String,
    pub tracks: Vec<LibraryTrack>,
}

/// Result of duplicate detection across libraries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateReport {
    pub duplicates: Vec<DuplicateGroup>,
    pub total_duplicate_tracks: usize,
    pub libraries_scanned: Vec<String>,
}

/// Characters that cannot appear in a folder or file name are replaced with
/// `_`; every other character is kept as written.
const INVALID_NAME_CHARS: [char; 10] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*', '\0'];

/// Case-insensitive key used to identify artists, albums and tracks.
/// Only case and surrounding whitespace are folded, so distinct names such as
/// `=` and `−` keep separate keys instead of collapsing into one album.
pub fn normalize_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| {
            if INVALID_NAME_CHARS.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseCompleteness {
    #[default]
    Unmatched,
    Partial,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryFolderInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub release_count: u32,
    pub track_count: u32,
    pub last_scanned_at: Option<String>,
    pub scan_status: String,
    pub scan_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryRelease {
    pub id: String,
    pub folder_id: String,
    pub folder_name: String,
    pub folder_path: String,
    pub album: String,
    pub album_artist: String,
    /// MusicBrainz's own disambiguation of this pressing, e.g. `deluxe edition`.
    #[serde(default)]
    pub album_disambig: Option<String>,
    /// Medium the release is, e.g. `CD` or `Digital Media`.
    #[serde(default)]
    pub media: Option<String>,
    /// Release country code, e.g. `GB`.
    #[serde(default)]
    pub country: Option<String>,
    pub year: Option<i32>,
    pub mbid: Option<String>,
    #[serde(default)]
    pub mb_release_group_id: Option<String>,
    pub path: String,
    pub track_count: u32,
    pub matched_track_count: u32,
    pub expected_track_count: Option<u32>,
    pub total_size: u64,
    /// Most common audio format in the release, e.g. `flac`.
    #[serde(default)]
    pub audio_format: Option<String>,
    /// Most common bitrate in kbps, when the release is lossy.
    #[serde(default)]
    pub bitrate: Option<u32>,
    /// Most common bit depth, when the release is lossless.
    #[serde(default)]
    pub bit_depth: Option<u32>,
    /// Most common sample rate in Hz, when known.
    #[serde(default)]
    pub sample_rate: Option<u32>,
    /// Whether the release mixes audio formats across its tracks.
    #[serde(default)]
    pub quality_varies: bool,
    pub completeness: ReleaseCompleteness,
    pub scanned_at: String,
    #[serde(default)]
    pub ignored: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTrackInfo {
    pub id: String,
    pub release_id: String,
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
    /// Audio format, e.g. `flac`, inferred from the file or beets.
    #[serde(default)]
    pub audio_format: Option<String>,
    /// Bitrate in kbps, when the track is lossy.
    #[serde(default)]
    pub bitrate: Option<u32>,
    /// Bit depth, when the track is lossless.
    #[serde(default)]
    pub bit_depth: Option<u32>,
    /// Sample rate in Hz, when known.
    #[serde(default)]
    pub sample_rate: Option<u32>,
    pub matched: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryMissingTrack {
    pub disc_number: Option<u32>,
    pub track_number: Option<u32>,
    pub title: String,
    pub artist: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryReleaseDetail {
    pub release: LibraryRelease,
    pub tracks: Vec<LibraryTrackInfo>,
    pub missing_tracks: Vec<LibraryMissingTrack>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryOverview {
    pub folders: Vec<LibraryFolderInfo>,
    pub releases: Vec<LibraryRelease>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryRemovalResult {
    pub removed_files: u32,
    pub failed_files: Vec<String>,
    pub removed_from_beets: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryAlbumRef {
    pub key: String,
    pub artist: String,
    pub album: String,
    #[serde(default)]
    pub mbid: Option<String>,
    #[serde(default)]
    pub track_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTrackRef {
    pub key: String,
    pub artist: String,
    pub title: String,
    #[serde(default)]
    pub album: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryMatchInfo {
    pub matched: bool,
    pub complete: bool,
    pub release_ids: Vec<String>,
    pub track_ids: Vec<String>,
    pub folder_ids: Vec<String>,
    pub folder_names: Vec<String>,
    pub track_count: u32,
    pub matched_track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryMatchQuery {
    #[serde(default)]
    pub albums: Vec<LibraryAlbumRef>,
    #[serde(default)]
    pub tracks: Vec<LibraryTrackRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryMatchResult {
    pub albums: BTreeMap<String, LibraryMatchInfo>,
    pub tracks: BTreeMap<String, LibraryMatchInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LibraryScanInterval {
    #[default]
    Off,
    TwoHours,
    SixHours,
    TwelveHours,
    OneDay,
    OneWeek,
}

impl LibraryScanInterval {
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::TwoHours => "2h",
            Self::SixHours => "6h",
            Self::TwelveHours => "12h",
            Self::OneDay => "1d",
            Self::OneWeek => "1w",
        }
    }

    pub fn from_key(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "2h" => Self::TwoHours,
            "6h" => Self::SixHours,
            "12h" => Self::TwelveHours,
            "1d" => Self::OneDay,
            "1w" => Self::OneWeek,
            _ => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Manual only",
            Self::TwoHours => "Every 2 hours",
            Self::SixHours => "Every 6 hours",
            Self::TwelveHours => "Every 12 hours",
            Self::OneDay => "Every day",
            Self::OneWeek => "Every week",
        }
    }

    pub fn duration(self) -> Option<std::time::Duration> {
        let hours = match self {
            Self::Off => return None,
            Self::TwoHours => 2,
            Self::SixHours => 6,
            Self::TwelveHours => 12,
            Self::OneDay => 24,
            Self::OneWeek => 24 * 7,
        };
        Some(std::time::Duration::from_secs(hours * 60 * 60))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryScanSettings {
    pub scan_interval: LibraryScanInterval,
    pub watch_enabled: bool,
    #[serde(default)]
    pub request_retry_interval: LibraryScanInterval,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTagTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub mb_track_id: Option<String>,
    pub mb_album_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTagCandidate {
    pub release_id: String,
    pub title: String,
    pub artist: String,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub track_count: u32,
    pub tracks: Vec<LibraryTagTrack>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTagFile {
    pub path: String,
    pub name: String,
    pub current_title: String,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub selected: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTagPreviewRequest {
    pub folder_id: String,
    #[serde(default)]
    pub release_id: Option<String>,
    pub path: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub is_track: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTagPreview {
    pub candidates: Vec<LibraryTagCandidate>,
    pub files: Vec<LibraryTagFile>,
    pub selected_candidate: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTagApplyTrack {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub mb_track_id: Option<String>,
    pub mb_album_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTagApplyRequest {
    pub folder_id: String,
    pub tracks: Vec<LibraryTagApplyTrack>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LibraryTagApplyResult {
    pub updated: u32,
    pub failures: Vec<String>,
    pub beets_available: bool,
    #[serde(default)]
    pub started: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DownloadActivity {
    pub albums: BTreeMap<String, String>,
    pub tracks: BTreeMap<String, String>,
    #[serde(default)]
    pub album_active_counts: BTreeMap<String, u32>,
}

pub fn album_activity_key(artist: &str, album: &str) -> String {
    format!("{}\u{1f}{}", normalize_key(artist), normalize_key(album))
}

pub fn track_activity_key(artist: &str, title: &str) -> String {
    format!("{}\u{1f}{}", normalize_key(artist), normalize_key(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_keys_fold_case_and_surrounding_whitespace_only() {
        assert_eq!(
            normalize_key("The Beatles - Hey Jude!"),
            normalize_key("  the beatles - hey jude!  ")
        );
        assert_ne!(
            normalize_key("The Beatles - Hey Jude!"),
            normalize_key("the beatles hey jude")
        );
        assert_ne!(normalize_key("Album One"), normalize_key("Album Two"));
    }

    #[test]
    fn library_keys_keep_symbol_only_names_distinct() {
        assert_ne!(normalize_key("="), normalize_key("−"));
        assert_eq!(normalize_key("="), "=");
        assert_eq!(normalize_key(" − "), "−");
        assert_eq!(normalize_key("   "), "");
    }

    #[test]
    fn library_keys_replace_characters_invalid_in_folder_and_file_names() {
        assert_eq!(normalize_key("AC/DC"), "ac_dc");
        assert_eq!(normalize_key("What?"), "what_");
        assert_eq!(normalize_key("A:B"), "a_b");
        assert_eq!(normalize_key("A《B》"), "a《b》");
    }

    #[test]
    fn activity_keys_ignore_case() {
        assert_eq!(
            album_activity_key("The Beatles", "Abbey Road!"),
            album_activity_key("the beatles", "abbey road!")
        );
        assert_eq!(
            track_activity_key("The Beatles", "Come Together"),
            track_activity_key("the beatles", "come together")
        );
        assert_ne!(
            album_activity_key("Ed Sheeran", "="),
            album_activity_key("Ed Sheeran", "−")
        );
    }

    #[test]
    fn scan_interval_keys_round_trip() {
        for interval in [
            LibraryScanInterval::Off,
            LibraryScanInterval::TwoHours,
            LibraryScanInterval::SixHours,
            LibraryScanInterval::TwelveHours,
            LibraryScanInterval::OneDay,
            LibraryScanInterval::OneWeek,
        ] {
            assert_eq!(LibraryScanInterval::from_key(interval.key()), interval);
        }
        assert_eq!(
            LibraryScanInterval::OneDay.duration(),
            Some(std::time::Duration::from_secs(24 * 3600))
        );
        assert_eq!(LibraryScanInterval::Off.duration(), None);
    }
}
