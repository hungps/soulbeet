use serde::{Deserialize, Serialize};

/// Metadata provider identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    MusicBrainz,
    LastFm,
    BrainzMash,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::MusicBrainz => write!(f, "musicbrainz"),
            Provider::LastFm => write!(f, "lastfm"),
            Provider::BrainzMash => write!(f, "brainzmash"),
        }
    }
}

impl std::str::FromStr for Provider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "musicbrainz" => Ok(Provider::MusicBrainz),
            "lastfm" => Ok(Provider::LastFm),
            "brainzmash" => Ok(Provider::BrainzMash),
            _ => Err(format!("Unknown provider: {}", s)),
        }
    }
}

/// Wrapper for search results that includes provider information.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResults {
    pub provider: Provider,
    pub results: Vec<SearchResult>,
}

/// Represents a search result which can be a track, a single album, or an
/// album group with its editions.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SearchResult {
    Track(Track),
    Album(Album),
    AlbumGroup(AlbumGroup),
}

/// A MusicBrainz-compatible server configured in the Config tab. Custom
/// servers are exposed as selectable metadata providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicBrainzServer {
    /// Stable provider identifier (`mb:<uuid>` for user-added rows).
    pub id: String,
    /// Display name shown in the provider picker.
    pub name: String,
    /// MusicBrainz ws/2 host, optionally with a port (`musicbrainz.example.com:8080`).
    pub host: String,
}

impl MusicBrainzServer {
    /// The pool entry shipped by default. Users can edit or remove it in the
    /// Config tab.
    pub fn defaults() -> Vec<Self> {
        vec![Self {
            id: "brainzmash".to_string(),
            name: "BrainzMash".to_string(),
            host: "api.brainzmash.cc".to_string(),
        }]
    }
}

/// A track from a metadata provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Provider-specific identifier for lookups.
    pub id: String,
    /// The title of the track.
    pub title: String,
    /// A formatted string of the artist(s).
    pub artist: String,
    /// Provider-specific identifier of the album the track belongs to.
    pub album_id: Option<String>,
    /// The title of the album the track belongs to.
    pub album_title: Option<String>,
    /// The release date of the album (YYYY-MM-DD).
    pub release_date: Option<String>,
    /// The duration of the track in a formatted MM:SS string.
    pub duration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disc_number: Option<u32>,
    /// The MusicBrainz recording ID, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mbid: Option<String>,
    /// The MusicBrainz release ID for the album, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_mbid: Option<String>,
}

/// An album from a metadata provider.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Album {
    /// Provider-specific identifier for lookups.
    pub id: String,
    /// The title of the album.
    pub title: String,
    /// A formatted string of the artist(s).
    pub artist: String,
    /// The release date of the album (YYYY-MM-DD).
    pub release_date: Option<String>,
    /// The MusicBrainz release ID, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mbid: Option<String>,
    /// URL to the album cover image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// Release country (ISO 3166-1 alpha-2), when the provider knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Medium summary of the release, e.g. `5×CD` or `Digital Media`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// Total track count across all mediums, when the provider reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_count: Option<u32>,
}

impl Album {
    /// Medium summary with the track count appended, e.g. `2×CD (15)`.
    pub fn medium_label(&self) -> Option<String> {
        let media = self.media.as_ref()?;
        Some(match self.track_count {
            Some(count) => format!("{media} ({count})"),
            None => media.clone(),
        })
    }
}

/// An album with its full track listing.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct AlbumWithTracks {
    pub album: Album,
    /// The first artist the provider credits, when it names more than one:
    /// `5 Seconds of Summer` for `5 Seconds of Summer feat. Julia Michaels`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_artist: Option<String>,
    pub tracks: Vec<Track>,
}

/// A logical album with its concrete editions. Provider-specific: MusicBrainz
/// maps this to a release group, Last.fm has no equivalent.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct AlbumGroup {
    /// Provider-specific identifier for lookups (MusicBrainz release-group MBID).
    pub id: String,
    /// The title of the album.
    pub title: String,
    /// A formatted string of the artist(s).
    pub artist: String,
    /// The release date of the earliest edition (YYYY-MM-DD).
    pub release_date: Option<String>,
    /// The MusicBrainz release-group ID, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mbid: Option<String>,
    /// URL to the album cover image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// Known editions of the album (pressings, deluxe, remastered, ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub editions: Vec<Album>,
}

/// Filters a metadata provider can apply to an album search. Empty vectors
/// mean "no filter".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchFilters {
    #[serde(default)]
    pub release_types: Vec<String>,
    #[serde(default)]
    pub secondary_types: Vec<String>,
    #[serde(default)]
    pub media_formats: Vec<String>,
}

impl SearchFilters {
    pub fn is_empty(&self) -> bool {
        self.release_types.is_empty()
            && self.secondary_types.is_empty()
            && self.media_formats.is_empty()
    }
}

/// The filter values a provider offers. Providers without album facets return
/// an empty list, which hides the filter UI for them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchFilterOptions {
    #[serde(default)]
    pub release_types: Vec<String>,
    #[serde(default)]
    pub secondary_types: Vec<String>,
    #[serde(default)]
    pub media_formats: Vec<String>,
}

impl SearchFilterOptions {
    pub fn is_supported(&self) -> bool {
        !self.release_types.is_empty()
            || !self.secondary_types.is_empty()
            || !self.media_formats.is_empty()
    }
}

/// Compare two MusicBrainz-style date strings ("YYYY", "YYYY-MM", "YYYY-MM-DD").
///
/// Years sort chronologically; within a year, the more specific date sorts
/// first, then the full string breaks ties.
pub fn compare_musicbrainz_dates(date1: Option<&str>, date2: Option<&str>) -> std::cmp::Ordering {
    let date_a = date1.filter(|s| !s.is_empty());
    let date_b = date2.filter(|s| !s.is_empty());

    match (date_a, date_b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(a), Some(b)) => {
            let year_a = &a[..a.len().min(4)];
            let year_b = &b[..b.len().min(4)];
            year_a
                .cmp(year_b)
                .then_with(|| b.len().cmp(&a.len()))
                .then_with(|| a.cmp(b))
        }
    }
}
