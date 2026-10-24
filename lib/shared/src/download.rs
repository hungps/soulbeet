use serde::{Deserialize, Serialize};

use crate::metadata::{Album, Track};

#[derive(Serialize, Clone, PartialEq, Deserialize, Debug, Default)]
pub struct DownloadQuery {
    pub album: Option<Album>,
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub backend: Option<String>,
    /// Metadata provider the album ID belongs to. Tracklists are resolved
    /// through it when the query carries no tracks yet.
    #[serde(default)]
    pub provider: Option<String>,
}

impl DownloadQuery {
    pub fn new(tracks: Vec<Track>) -> Self {
        Self {
            album: None,
            tracks,
            backend: None,
            provider: None,
        }
    }

    pub fn album(mut self, album: Album) -> Self {
        self.album = Some(album);
        self
    }

    pub fn backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Identity of the release this query targets. Downloads issued for the
    /// same release join one request session.
    pub fn request_context(&self) -> DownloadRequestContext {
        if let Some(album) = &self.album {
            return DownloadRequestContext {
                key: format!("album:{}", album.id),
                label: album.title.clone(),
                artist: Some(album.artist.clone()),
            };
        }

        if let Some(track) = self.tracks.first() {
            let key = track
                .album_id
                .as_ref()
                .map(|id| format!("album:{id}"))
                .unwrap_or_else(|| format!("track:{}", track.id));
            return DownloadRequestContext {
                key,
                label: track
                    .album_title
                    .clone()
                    .unwrap_or_else(|| track.title.clone()),
                artist: Some(track.artist.clone()),
            };
        }

        DownloadRequestContext::default()
    }
}

/// Identity of the release a manual or automatic download belongs to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DownloadRequestContext {
    /// Stable key of the release (metadata album ID when known).
    pub key: String,
    /// Human-readable release title.
    pub label: String,
    /// Release artist, when known.
    #[serde(default)]
    pub artist: Option<String>,
}

/// A persisted request session shown as one expandable card in the transfer panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadRequestSummary {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub artist: Option<String>,
    pub target_folder: String,
    /// `active`, `stopped` or `completed`.
    pub state: String,
    pub tracks: Vec<DownloadRequestTrack>,
}

/// A request that has not reached the transfer stage yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingRequestSummary {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub artist: Option<String>,
    /// `searching` while a source search runs, `requested` while queued for retry.
    pub state: String,
    #[serde(default)]
    pub track_count: usize,
    #[serde(default)]
    pub attempts: u32,
}

/// Persisted state of one track inside a request session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadRequestTrack {
    pub item: String,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub size: u64,
    pub quality: String,
    pub transferred: u64,
    pub percent: f64,
    pub speed: f64,
    pub state: DownloadState,
    pub error: Option<String>,
    pub transfer_id: Option<String>,
}

impl From<Track> for DownloadQuery {
    fn from(track: Track) -> Self {
        Self::new(vec![track])
    }
}

/// A downloadable item from a search result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadableItem {
    /// Unique identifier for this item (backend-specific format)
    pub id: String,
    /// Source identifier (e.g., username for P2P, service for streaming)
    pub source: String,
    /// Display name for the track
    pub title: String,
    /// Artist name
    pub artist: String,
    /// Album name
    pub album: String,
    /// File size in bytes (if known)
    pub size: Option<u64>,
    /// Duration in seconds (if known)
    pub duration: Option<u32>,
    /// Quality descriptor (e.g., "FLAC", "320kbps MP3")
    pub quality: String,
    /// Quality score 0.0-1.0 for ranking
    pub quality_score: f64,
    /// Backend-specific data (JSON serialized)
    #[serde(default)]
    pub backend_data: Option<String>,
}

/// A group of downloadable items (e.g., album from one source)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadableGroup {
    /// Source identifier
    pub source: String,
    /// Group identifier (e.g., folder path, album ID)
    pub group_id: String,
    /// Display title for the group
    pub title: String,
    /// Artist name
    pub artist: Option<String>,
    /// Number of items in group
    pub item_count: usize,
    /// Total size in bytes
    pub total_size: u64,
    /// Items in this group
    pub items: Vec<DownloadableItem>,
    /// Dominant quality in the group
    pub quality: String,
    /// Overall score for ranking
    pub score: f64,
}

impl DownloadableGroup {
    pub fn size_mb(&self) -> u64 {
        self.total_size / (1024 * 1024)
    }
}

/// State of a search operation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SearchState {
    InProgress,
    Completed,
    NotFound,
    TimedOut,
}

/// Result of a download search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub search_id: String,
    pub groups: Vec<DownloadableGroup>,
    pub has_more: bool,
    pub state: SearchState,
}

/// State of a download operation
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum DownloadState {
    Queued,
    InProgress,
    Completed,
    Importing,
    Imported,
    ImportSkipped,
    Failed(String),
    Cancelled,
    /// The whole request was stopped by the user and can be resumed.
    Stopped,
}

impl DownloadState {
    /// Whether this state is final for a request session. Downloaded files
    /// still waiting to be imported are not final.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            DownloadState::Imported
                | DownloadState::ImportSkipped
                | DownloadState::Failed(_)
                | DownloadState::Cancelled
                | DownloadState::Stopped
        )
    }
}

/// Progress of a single download
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadProgress {
    /// Unique identifier
    pub id: String,
    /// Source identifier
    pub source: String,
    /// Item being downloaded (title or filename)
    pub item: String,
    /// Total size in bytes
    pub size: u64,
    /// Bytes transferred
    pub transferred: u64,
    /// Current state
    pub state: DownloadState,
    /// Progress percentage (0.0-100.0)
    pub percent: f64,
    /// Average download speed in bytes per second
    #[serde(default)]
    pub speed: f64,
    /// Error message if failed
    pub error: Option<String>,
    /// Which download backend owns this transfer
    #[serde(default)]
    pub backend: Option<String>,
    /// Groups this download with others from the same batch
    #[serde(default)]
    pub batch_id: Option<String>,
    /// Human-readable batch label (album name)
    #[serde(default)]
    pub batch_label: Option<String>,
}

impl DownloadProgress {
    pub fn queued(id: String, source: String, item: String, size: u64) -> Self {
        Self {
            id,
            source,
            item,
            size,
            transferred: 0,
            state: DownloadState::Queued,
            percent: 0.0,
            speed: 0.0,
            error: None,
            backend: None,
            batch_id: None,
            batch_label: None,
        }
    }

    pub fn failed(id: String, source: String, item: String, error: String) -> Self {
        Self {
            id,
            source,
            item,
            size: 0,
            transferred: 0,
            state: DownloadState::Failed(error.clone()),
            percent: 0.0,
            speed: 0.0,
            error: Some(error),
            backend: None,
            batch_id: None,
            batch_label: None,
        }
    }

    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    pub fn with_state(mut self, state: DownloadState) -> Self {
        self.state = state;
        self
    }

    pub fn with_batch(mut self, batch_id: String, batch_label: String) -> Self {
        self.batch_id = Some(batch_id);
        self.batch_label = Some(batch_label);
        self
    }
}

/// Response from queueing downloads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedDownload {
    pub id: String,
    pub source: String,
    pub item: String,
    pub size: u64,
    pub error: Option<String>,
}

impl QueuedDownload {
    pub fn success(id: String, source: String, item: String, size: u64) -> Self {
        Self {
            id,
            source,
            item,
            size,
            error: None,
        }
    }

    pub fn failed(id: String, source: String, item: String, error: String) -> Self {
        Self {
            id,
            source,
            item,
            size: 0,
            error: Some(error),
        }
    }

    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

/// Wrapper for all download-related WebSocket events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DownloadEvent {
    /// Standard download progress updates
    Progress(Vec<DownloadProgress>),
    /// A track moved to another peer. The client keys rows by remote path and
    /// every peer serves the track under its own, so a plain progress update
    /// would leave the abandoned transfer's row sitting next to the retry's.
    /// Failover rebinds one download rather than starting a second, and the
    /// panel has to show it that way.
    Replaced {
        previous_item: String,
        entry: DownloadProgress,
    },
    /// Auto-download lifecycle events (displayed inline on search rows, not in download panel)
    AutoDownload(AutoDownloadEvent),
    /// A request was removed together with all of its tracks.
    RequestRemoved {
        request_id: String,
        items: Vec<String>,
    },
}

/// Events from the auto-download pipeline, sent over WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AutoDownloadEvent {
    /// Backend search started
    Searching {
        batch_id: String,
        query: String,
        backend_count: usize,
    },
    /// Results scored and ranked
    ScoringResults {
        batch_id: String,
        result_count: usize,
        best_score: f64,
    },
    /// Source selected for download
    PickedSource {
        batch_id: String,
        source: String,
        score: f64,
        quality: String,
        track_count: usize,
    },
    /// Download queued with the picked source
    Downloading { batch_id: String },
    /// Best score below threshold, client should show manual source picker
    FallbackToManual {
        batch_id: String,
        results: Vec<DownloadableGroup>,
        best_score: f64,
        threshold: f64,
    },
    /// Auto-download pipeline failed
    Failed { batch_id: String, error: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Album, Track};

    fn track(album_id: Option<&str>) -> Track {
        Track {
            id: "recording".to_string(),
            title: "A Track".to_string(),
            artist: "An Artist".to_string(),
            album_id: album_id.map(str::to_string),
            album_title: Some("An Album".to_string()),
            release_date: None,
            duration: None,
            track_number: None,
            disc_number: None,
            mbid: None,
            release_mbid: None,
        }
    }

    #[test]
    fn album_queries_share_the_release_key_across_editions() {
        let album = |id: &str| Album {
            id: id.to_string(),
            title: "An Album".to_string(),
            artist: "An Artist".to_string(),
            release_date: None,
            mbid: None,
            cover_url: None,
            country: None,
            media: None,
            track_count: None,
        };

        let first = DownloadQuery::new(Vec::new()).album(album("release-1"));
        let second = DownloadQuery::new(Vec::new()).album(album("release-2"));
        assert_ne!(first.request_context().key, second.request_context().key);
    }

    #[test]
    fn track_queries_use_the_album_identity_when_known() {
        let context = DownloadQuery::from(track(Some("release-1"))).request_context();
        assert_eq!(context.key, "album:release-1");
        assert_eq!(context.label, "An Album");
    }

    #[test]
    fn track_queries_fall_back_to_the_recording_identity() {
        let context = DownloadQuery::from(track(None)).request_context();
        assert_eq!(context.key, "track:recording");
    }
}
