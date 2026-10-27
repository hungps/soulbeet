use musicbrainz_rs::{
    entity::{
        artist_credit::ArtistCredit,
        recording::Recording,
        release::Release,
        release_group::{ReleaseGroup, ReleaseGroupPrimaryType, ReleaseGroupSecondaryType},
    },
    Fetch, MusicBrainzClient, Search,
};
use shared::metadata::{
    compare_musicbrainz_dates, Album, AlbumGroup, AlbumWithTracks, SearchFilterOptions,
    SearchFilters, SearchResult, Track,
};
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    future::Future,
    sync::OnceLock,
    time::Duration,
};
use tokio::time::sleep;
use tracing::{info, warn};

/// Timeout for individual MusicBrainz requests (15 seconds)
const REQUEST_TIMEOUT_SECS: u64 = 15;

/// Maximum retries for transient errors
const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (milliseconds)
const BASE_DELAY_MS: u64 = 500;

/// Maximum backoff delay cap (milliseconds)
const MAX_BACKOFF_MS: u64 = 5000;

/// Releases fetched per album search. Release groups are assembled client side,
/// so this needs to be well above the number of groups we return.
const ALBUM_FETCH_LIMIT: u8 = 100;

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string()
}

fn build_client(host: &str) -> MusicBrainzClient {
    let mut client = MusicBrainzClient::new("Lidarr/3.1.6.5078")
        .expect("Failed to create MusicBrainz client - invalid user agent format");
    client.musicbrainz_domain = normalize_host(host);
    client
}

/// Official MusicBrainz server. Alternate servers are configured in the
/// Config tab and built per provider instance.
fn musicbrainz_client() -> &'static MusicBrainzClient {
    static CLIENT: OnceLock<MusicBrainzClient> = OnceLock::new();
    CLIENT.get_or_init(|| build_client("musicbrainz.org"))
}

/// Formats the artist credits into a single, comma-separated string.
fn format_artist_credit(credits: &Option<Vec<ArtistCredit>>) -> String {
    credits
        .as_ref()
        .map(|credits| {
            credits
                .iter()
                .map(|credit| credit.name.clone())
                .collect::<Vec<String>>()
                .join(", ")
        })
        .unwrap_or_else(|| "Unknown Artist".to_string())
}

/// The first artist of a credit list: MusicBrainz names every artist of a
/// collaboration and the first one is the act the release is filed under.
fn main_artist(credits: &Option<Vec<ArtistCredit>>) -> Option<String> {
    credits
        .as_ref()
        .and_then(|credits| credits.first())
        .map(|credit| credit.name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// Formats a duration from milliseconds to a MM:SS string.
fn format_duration(duration_ms: &Option<u32>) -> Option<String> {
    duration_ms.map(|ms| {
        let total_seconds = ms / 1000;
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{minutes:02}:{seconds:02}")
    })
}

/// Check if an error is retryable (transient network/server issues)
fn is_retryable_error(error: &musicbrainz_rs::Error) -> bool {
    let error_str = format!("{:?}", error);
    let error_lower = error_str.to_lowercase();

    // Retry on timeout, connection, and 5xx server errors
    if error_lower.contains("timeout")
        || error_lower.contains("connection")
        || error_lower.contains("timed out")
        || error_lower.contains("503")
        || error_lower.contains("502")
        || error_lower.contains("500")
        || error_lower.contains("429") // Rate limited - should retry after backoff
        || error_lower.contains("service unavailable")
    {
        return true;
    }

    // Don't retry client errors (4xx except 429)
    if error_lower.contains("400")
        || error_lower.contains("401")
        || error_lower.contains("403")
        || error_lower.contains("404")
        || error_lower.contains("bad request")
        || error_lower.contains("not found")
        || error_lower.contains("unauthorized")
    {
        return false;
    }

    // Default to retrying for unknown errors (network issues, etc.)
    true
}

/// Retries an async operation with exponential backoff and request timeout.
/// Only retries transient errors (network issues, timeouts, 5xx responses).
/// Does NOT retry client errors (4xx) or permanent failures.
async fn with_retry<T, F, Fut>(
    operation_name: &str,
    mut operation: F,
) -> Result<T, musicbrainz_rs::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, musicbrainz_rs::Error>>,
{
    let mut last_error = None;

    for attempt in 0..MAX_RETRIES {
        // Respect MusicBrainz rate limit (1 req/sec)
        crate::http::mb_rate_limit().await;

        // Apply timeout to each request
        let result =
            tokio::time::timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS), operation()).await;

        match result {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(e)) => {
                // Check if this error is retryable
                if !is_retryable_error(&e) {
                    warn!(
                        "{} failed with non-retryable error: {:?}",
                        operation_name, e
                    );
                    return Err(e);
                }

                last_error = Some(e);
                if attempt < MAX_RETRIES - 1 {
                    let delay = std::cmp::min(BASE_DELAY_MS * 2u64.pow(attempt), MAX_BACKOFF_MS);
                    warn!(
                        "{} failed (attempt {}/{}), retrying in {}ms: {:?}",
                        operation_name,
                        attempt + 1,
                        MAX_RETRIES,
                        delay,
                        last_error
                    );
                    sleep(Duration::from_millis(delay)).await;
                }
            }
            Err(_timeout) => {
                // Request timed out
                warn!(
                    "{} timed out after {}s (attempt {}/{})",
                    operation_name,
                    REQUEST_TIMEOUT_SECS,
                    attempt + 1,
                    MAX_RETRIES
                );
                last_error = Some(musicbrainz_rs::Error::MaxRetriesExceeded);
                if attempt < MAX_RETRIES - 1 {
                    let delay = std::cmp::min(BASE_DELAY_MS * 2u64.pow(attempt), MAX_BACKOFF_MS);
                    sleep(Duration::from_millis(delay)).await;
                }
            }
        }
    }

    match last_error {
        Some(e) => Err(e),
        None => Err(musicbrainz_rs::Error::MaxRetriesExceeded),
    }
}

/// An enumeration to specify the type of search.
#[derive(Debug)]
pub enum SearchType {
    Track,
    Album,
}

/// One user word as a Lucene term. Reserved characters (`+`, `-`, `(`, `:`, ...)
/// are quoted so they stay literal instead of becoming query operators.
fn lucene_term(word: &str) -> String {
    let escaped = word.replace('\\', "\\\\").replace('"', "\\\"");
    if escaped.chars().all(char::is_alphanumeric) {
        escaped
    } else {
        format!("\"{escaped}\"")
    }
}

/// A user value as an AND-ed Lucene clause, e.g. `Metaphor ReFantazio` becomes
/// `release:(Metaphor AND ReFantazio)`. Every word has to match, in any order;
/// punctuation-only words are dropped unless they are the whole value, so `+`
/// still finds Ed Sheeran's "+".
fn field_clause(field: &str, value: &str) -> Option<String> {
    let mut terms: Vec<String> = value
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .map(lucene_term)
        .collect();
    if terms.is_empty() {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        terms.push(lucene_term(value));
    }
    Some(format!("{field}:({})", terms.join(" AND ")))
}

/// MusicBrainz search parameters for the given clauses.
///
/// The whole Lucene query is percent-encoded: MusicBrainz decodes it before
/// parsing, so a `+` left in the URL (Ed Sheeran's "+") would arrive as a
/// space and silently empty the clause it belongs to.
fn search_params(clauses: &[String]) -> Option<String> {
    let query = clauses.join(" AND ");
    (!query.is_empty()).then(|| {
        format!(
            "query={}",
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
        )
    })
}

/// Albums matching the query text, or every album of the artist when the
/// search box is empty.
fn album_search_params(artist: Option<&str>, query: &str) -> Option<String> {
    let mut clauses = Vec::new();
    if let Some(artist) = field_clause("artist", artist.unwrap_or_default()) {
        clauses.push(artist);
    }
    if let (Some(release), Some(alias)) =
        (field_clause("release", query), field_clause("alias", query))
    {
        clauses.push(format!("({release} OR {alias})"));
    }
    search_params(&clauses)
}

/// Tracks matching the query text, or every track of the artist when the
/// search box is empty.
fn track_search_params(artist: Option<&str>, query: &str) -> Option<String> {
    let mut clauses = Vec::new();
    if let Some(artist) = field_clause("artist", artist.unwrap_or_default()) {
        clauses.push(artist);
    }
    if let Some(recording) = field_clause("recording", query) {
        clauses.push(recording);
    }
    search_params(&clauses)
}

fn query_tokens(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() > 2)
        .map(str::to_string)
        .collect()
}

fn matched_tokens(title: &str, tokens: &[String]) -> usize {
    let title = title.to_lowercase();
    tokens
        .iter()
        .filter(|token| title.contains(token.as_str()))
        .count()
}

fn album_title(group_title: &str, release_title: &str, tokens: &[String]) -> String {
    if tokens.is_empty() || matched_tokens(group_title, tokens) > 0 {
        group_title.to_string()
    } else {
        release_title.to_string()
    }
}

fn edition_title(release: &Release) -> String {
    match release.disambiguation.as_deref().map(str::trim) {
        Some(comment) if !comment.is_empty() && !is_medium_disambiguation(comment) => {
            format!("{} ({})", release.title, comment)
        }
        _ => release.title.clone(),
    }
}

fn is_medium_disambiguation(comment: &str) -> bool {
    let letters: String = comment
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    let base = letters.trim_matches(|c: char| c.is_ascii_digit());
    matches!(
        base,
        "cd" | "disc"
            | "disk"
            | "dvd"
            | "sacd"
            | "bluray"
            | "vinyl"
            | "lp"
            | "cassette"
            | "medium"
            | "digitalmedia"
    )
}

fn sort_editions(editions: &mut [Album], tokens: &[String]) {
    editions.sort_by(|a, b| {
        matched_tokens(&b.title, tokens)
            .cmp(&matched_tokens(&a.title, tokens))
            .then_with(|| {
                compare_musicbrainz_dates(a.release_date.as_deref(), b.release_date.as_deref())
            })
            .then_with(|| a.title.cmp(&b.title))
    });
}

fn media_label(release: &Release) -> Option<String> {
    let formats: Vec<&str> = release
        .media
        .as_ref()?
        .iter()
        .filter_map(|medium| medium.format.as_deref())
        .collect();
    let first = formats.first()?;
    if formats.iter().all(|format| format == first) {
        if formats.len() > 1 {
            Some(format!("{}×{}", formats.len(), first))
        } else {
            Some(first.to_string())
        }
    } else {
        Some(formats.join(" + "))
    }
}

fn release_track_count(release: &Release) -> Option<u32> {
    let media = release.media.as_ref()?;
    let total: u32 = media.iter().map(|medium| medium.track_count).sum();
    (total > 0).then_some(total)
}

fn primary_type_name(primary_type: Option<&ReleaseGroupPrimaryType>) -> Option<&'static str> {
    Some(match primary_type? {
        ReleaseGroupPrimaryType::Album => "Album",
        ReleaseGroupPrimaryType::Single => "Single",
        ReleaseGroupPrimaryType::Ep => "EP",
        ReleaseGroupPrimaryType::Broadcast => "Broadcast",
        ReleaseGroupPrimaryType::Other => "Other",
        _ => return None,
    })
}

fn secondary_type_name(secondary_type: &ReleaseGroupSecondaryType) -> &'static str {
    match secondary_type {
        ReleaseGroupSecondaryType::AudioDrama => "Audio drama",
        ReleaseGroupSecondaryType::Audiobook => "Audiobook",
        ReleaseGroupSecondaryType::Compilation => "Compilation",
        ReleaseGroupSecondaryType::DjMix => "DJ-mix",
        ReleaseGroupSecondaryType::Demo => "Demo",
        ReleaseGroupSecondaryType::Interview => "Interview",
        ReleaseGroupSecondaryType::Live => "Live",
        ReleaseGroupSecondaryType::MixtapeStreet => "Mixtape/Street",
        ReleaseGroupSecondaryType::Remix => "Remix",
        ReleaseGroupSecondaryType::Soundtrack => "Soundtrack",
        ReleaseGroupSecondaryType::Spokenword => "Spokenword",
        _ => "Other",
    }
}

fn matches_any(wanted: &[String], value: &str) -> bool {
    wanted
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(value))
}

fn format_matches(release: &Release, filters: &SearchFilters) -> bool {
    if filters.media_formats.is_empty() {
        return true;
    }
    release.media.as_ref().is_some_and(|media| {
        media
            .iter()
            .filter_map(|medium| medium.format.as_deref())
            .any(|format| matches_any(&filters.media_formats, format))
    })
}

fn release_matches(release: &Release, group: &ReleaseGroup, filters: &SearchFilters) -> bool {
    let primary_match = if filters.release_types.is_empty() {
        None
    } else {
        Some(
            primary_type_name(group.primary_type.as_ref())
                .is_some_and(|primary| matches_any(&filters.release_types, primary)),
        )
    };

    let secondary_match =
        if filters.secondary_types.is_empty() {
            None
        } else {
            Some(group.secondary_types.iter().any(|secondary| {
                matches_any(&filters.secondary_types, secondary_type_name(secondary))
            }))
        };

    let type_match = match (primary_match, secondary_match) {
        (None, None) => true,
        (Some(primary), None) => primary,
        (None, Some(secondary)) => secondary,
        (Some(primary), Some(secondary)) => primary || secondary,
    };

    type_match && format_matches(release, filters)
}

fn filter_options() -> SearchFilterOptions {
    SearchFilterOptions {
        release_types: ["Album", "Single", "EP", "Broadcast", "Other"]
            .map(str::to_string)
            .to_vec(),
        secondary_types: [
            "Compilation",
            "Soundtrack",
            "Spokenword",
            "Interview",
            "Audiobook",
            "Audio drama",
            "Live",
            "Remix",
            "DJ-mix",
            "Mixtape/Street",
            "Demo",
        ]
        .map(str::to_string)
        .to_vec(),
        media_formats: [
            "CD",
            "Digital Media",
            "Vinyl",
            "12\" Vinyl",
            "10\" Vinyl",
            "7\" Vinyl",
            "Cassette",
            "SACD",
            "DVD",
            "Blu-ray",
        ]
        .map(str::to_string)
        .to_vec(),
    }
}

/// Tracks as printed on this release. MusicBrainz keeps the release's track
/// title/credit separate from the linked recording, which carries the canonical
/// (often original-language) title, so the release values win here.
fn release_tracks(release: &Release) -> Vec<Track> {
    let mut tracks = Vec::new();

    if let Some(media) = &release.media {
        for (medium_index, medium) in media.iter().enumerate() {
            let Some(release_tracks) = &medium.tracks else {
                continue;
            };
            for track in release_tracks {
                let Some(recording) = &track.recording else {
                    continue;
                };
                let credits = track
                    .artist_credit
                    .clone()
                    .or_else(|| recording.artist_credit.clone());

                tracks.push(Track {
                    id: recording.id.clone(),
                    title: track.title.clone(),
                    artist: format_artist_credit(&credits),
                    album_id: Some(release.id.clone()),
                    album_title: Some(release.title.clone()),
                    release_date: release.date.as_ref().map(|d| d.0.clone()),
                    duration: format_duration(&track.length.or(recording.length)),
                    track_number: Some(track.position),
                    disc_number: Some(medium_index as u32 + 1),
                    mbid: Some(recording.id.clone()),
                    release_mbid: Some(release.id.clone()),
                });
            }
        }
    }

    tracks
}

type EditionKey = (String, Option<String>, Option<String>);

struct GroupAccum {
    title: String,
    artist: String,
    editions: HashMap<EditionKey, Album>,
}

pub struct MusicBrainzProvider {
    id: &'static str,
    name: &'static str,
    client: MusicBrainzClient,
}

impl MusicBrainzProvider {
    pub fn new() -> Self {
        Self {
            id: "musicbrainz",
            name: "MusicBrainz",
            client: musicbrainz_client().clone(),
        }
    }

    /// A MusicBrainz-compatible server configured in the Config tab. It keeps
    /// the MusicBrainz identity because the ws/2 data model is identical; the
    /// configured name is only used by the provider picker.
    pub fn from_host(host: &str) -> Self {
        Self {
            id: "musicbrainz",
            name: "MusicBrainz",
            client: build_client(host),
        }
    }

    async fn search(
        &self,
        artist: Option<&str>,
        query: &str,
        search_type: SearchType,
        filters: &SearchFilters,
        limit: usize,
    ) -> Result<Vec<SearchResult>, musicbrainz_rs::Error> {
        let client = &self.client;
        let mut results = Vec::new();

        info!(
            "Starting {:?} search for query: '{}', artist: '{:?}'",
            search_type, query, artist
        );

        match search_type {
            SearchType::Track => {
                let Some(params) = track_search_params(artist, query) else {
                    return Ok(results);
                };
                let limit = limit.min(100) as u8;
                let search_results = with_retry("MusicBrainz track search", || {
                    let params = params.clone();
                    async move {
                        Recording::search(params)
                            .limit(limit)
                            .with_releases()
                            .with_ratings()
                            .execute_with_client(client)
                            .await
                    }
                })
                .await?;

                // Sort by rating (descending) - higher rated recordings first
                let mut recordings: Vec<_> = search_results.entities;
                recordings.sort_by(|a, b| {
                    let a_rating = a.rating.as_ref().and_then(|r| r.value).unwrap_or(0.0);
                    let b_rating = b.rating.as_ref().and_then(|r| r.value).unwrap_or(0.0);
                    b_rating
                        .partial_cmp(&a_rating)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                let mut unique_tracks = HashSet::new();

                for recording in recordings {
                    let artist_name = format_artist_credit(&recording.artist_credit);
                    let album_title = recording
                        .releases
                        .as_ref()
                        .and_then(|r| r.first())
                        .map(|r| r.title.clone())
                        .unwrap_or_default();

                    // Use a combination of title, artist, and album to define a unique track
                    let key = (
                        recording.title.to_lowercase(),
                        artist_name.to_lowercase(),
                        album_title.to_lowercase(),
                    );

                    if !unique_tracks.contains(&key) {
                        let first_release = recording.releases.as_ref().and_then(|r| r.first());
                        let track = Track {
                            id: recording.id.clone(),
                            title: recording.title.clone(),
                            artist: artist_name.clone(),
                            album_id: first_release.map(|release| release.id.clone()),
                            album_title: first_release.map(|r| r.title.clone()),
                            release_date: first_release.and_then(|r| r.date.clone().map(|d| d.0)),
                            duration: format_duration(&recording.length),
                            track_number: None,
                            disc_number: None,
                            mbid: Some(recording.id.clone()),
                            release_mbid: first_release.map(|r| r.id.clone()),
                        };
                        unique_tracks.insert(key);
                        results.push(SearchResult::Track(track));
                    }
                }
            }
            SearchType::Album => {
                let Some(params) = album_search_params(artist, query) else {
                    return Ok(results);
                };
                let search_results = with_retry("MusicBrainz album search", || {
                    let params = params.clone();
                    async move {
                        Release::search(params)
                            .limit(ALBUM_FETCH_LIMIT)
                            .with_release_groups()
                            .execute_with_client(client)
                            .await
                    }
                })
                .await?;

                let tokens = query_tokens(query);
                let mut groups: HashMap<String, GroupAccum> = HashMap::new();
                let mut rank: HashMap<String, usize> = HashMap::new();

                for release in search_results.entities {
                    let Some(group) = release.release_group.as_ref() else {
                        continue;
                    };
                    if !release_matches(&release, group, filters) {
                        continue;
                    }

                    let group_id = group.id.clone();
                    if !rank.contains_key(&group_id) {
                        let position = rank.len();
                        rank.insert(group_id.clone(), position);
                    }

                    let edition = Album {
                        id: release.id.clone(),
                        title: edition_title(&release),
                        artist: format_artist_credit(&release.artist_credit),
                        release_date: release.date.as_ref().map(|d| d.0.clone()),
                        mbid: Some(release.id.clone()),
                        cover_url: Some(format!(
                            "https://coverartarchive.org/release/{}/front-250",
                            release.id
                        )),
                        country: release.country.clone(),
                        media: media_label(&release),
                        track_count: release_track_count(&release),
                    };

                    let accum = groups.entry(group_id).or_insert_with(|| GroupAccum {
                        title: group.title.clone(),
                        // Release search results carry the credited artists on the
                        // release, not on the nested release group.
                        artist: format_artist_credit(
                            &group
                                .artist_credit
                                .clone()
                                .or_else(|| release.artist_credit.clone()),
                        ),
                        editions: HashMap::new(),
                    });

                    let key = (
                        edition.title.clone(),
                        edition.country.clone(),
                        edition.media.clone(),
                    );
                    match accum.editions.entry(key) {
                        Entry::Vacant(slot) => {
                            slot.insert(edition);
                        }
                        Entry::Occupied(mut slot) => {
                            let existing_date = slot.get().release_date.clone();
                            if compare_musicbrainz_dates(
                                existing_date.as_deref(),
                                edition.release_date.as_deref(),
                            ) == std::cmp::Ordering::Greater
                            {
                                slot.insert(edition);
                            }
                        }
                    }
                }

                let mut grouped = Vec::with_capacity(groups.len());
                for (group_id, accum) in groups {
                    let mut editions: Vec<Album> = accum.editions.into_values().collect();
                    sort_editions(&mut editions, &tokens);
                    let Some(primary_edition) = editions.first() else {
                        continue;
                    };
                    grouped.push(SearchResult::AlbumGroup(AlbumGroup {
                        cover_url: Some(format!(
                            "https://coverartarchive.org/release-group/{group_id}/front-250"
                        )),
                        id: group_id.clone(),
                        title: album_title(&accum.title, &primary_edition.title, &tokens),
                        artist: accum.artist,
                        release_date: primary_edition.release_date.clone(),
                        mbid: Some(group_id),
                        editions,
                    }));
                }

                grouped.sort_by(|a, b| {
                    let (SearchResult::AlbumGroup(a), SearchResult::AlbumGroup(b)) = (a, b) else {
                        return std::cmp::Ordering::Equal;
                    };
                    let a_rank = rank.get(&a.id).copied().unwrap_or(usize::MAX);
                    let b_rank = rank.get(&b.id).copied().unwrap_or(usize::MAX);
                    a_rank.cmp(&b_rank).then_with(|| {
                        compare_musicbrainz_dates(
                            a.release_date.as_deref(),
                            b.release_date.as_deref(),
                        )
                    })
                });
                grouped.truncate(limit);
                results.extend(grouped);
            }
        }

        Ok(results)
    }

    async fn fetch_album(
        &self,
        release_id: &str,
    ) -> Result<AlbumWithTracks, musicbrainz_rs::Error> {
        let client = &self.client;

        // Fetch the release with recordings (tracks) and artist credits for the tracks.
        let release = with_retry("MusicBrainz album fetch", || async {
            Release::fetch()
                .id(release_id)
                .with_recordings()
                .with_artist_credits()
                .execute_with_client(client)
                .await
        })
        .await?;

        let tracks = release_tracks(&release);

        let media = media_label(&release);
        let track_count = release_track_count(&release);
        let album = Album {
            id: release.id.clone(),
            title: edition_title(&release),
            artist: format_artist_credit(&release.artist_credit),
            release_date: release.date.map(|d| d.0),
            mbid: Some(release.id),
            cover_url: None,
            country: release.country.clone(),
            media,
            track_count,
        };

        Ok(AlbumWithTracks {
            main_artist: main_artist(&release.artist_credit),
            album,
            tracks,
        })
    }
}

impl Default for MusicBrainzProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl crate::MetadataProvider for MusicBrainzProvider {
    fn id(&self) -> &'static str {
        self.id
    }

    fn name(&self) -> &'static str {
        self.name
    }

    async fn search_albums(
        &self,
        artist: Option<&str>,
        query: &str,
        filters: &SearchFilters,
        limit: usize,
    ) -> crate::error::Result<Vec<SearchResult>> {
        self.search(artist, query, SearchType::Album, filters, limit)
            .await
            .map_err(|e| crate::error::SoulseekError::Api {
                status: 500,
                message: e.to_string(),
            })
    }

    async fn search_tracks(
        &self,
        artist: Option<&str>,
        query: &str,
        limit: usize,
    ) -> crate::error::Result<Vec<SearchResult>> {
        self.search(
            artist,
            query,
            SearchType::Track,
            &SearchFilters::default(),
            limit,
        )
        .await
        .map_err(|e| crate::error::SoulseekError::Api {
            status: 500,
            message: e.to_string(),
        })
    }

    fn search_filter_options(&self) -> SearchFilterOptions {
        filter_options()
    }

    async fn get_album(&self, id: &str) -> crate::error::Result<AlbumWithTracks> {
        self.fetch_album(id)
            .await
            .map_err(|e| crate::error::SoulseekError::Api {
                status: 500,
                message: e.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(id: &str, title: &str, date: &str, status: &str, formats: &[&str]) -> Release {
        let media: Vec<serde_json::Value> = formats
            .iter()
            .map(|format| serde_json::json!({ "format": format, "track-count": 10 }))
            .collect();
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": title,
            "date": date,
            "status": status,
            "media": media,
        }))
        .unwrap()
    }

    fn release_group(primary: &str, secondary: &[&str]) -> ReleaseGroup {
        serde_json::from_value(serde_json::json!({
            "id": "group",
            "title": "Metaphor: ReFantazio Original Soundtrack",
            "primary-type": primary,
            "secondary-types": secondary,
        }))
        .unwrap()
    }

    #[test]
    fn edition_titles_leave_out_a_medium_disambiguation() {
        let mut release = release("r", "Album", "2024", "Official", &["CD"]);
        release.disambiguation = Some("CD".to_string());
        assert_eq!(edition_title(&release), "Album");
        release.disambiguation = Some("Digital Media".to_string());
        assert_eq!(edition_title(&release), "Album");
        release.disambiguation = Some("deluxe edition".to_string());
        assert_eq!(edition_title(&release), "Album (deluxe edition)");
    }

    #[test]
    fn release_track_titles_win_over_recording_titles() {
        let release: Release = serde_json::from_value(serde_json::json!({
            "id": "release-us",
            "title": "Metaphor: ReFantazio Original Soundtrack",
            "date": "2025-01-15",
            "media": [{
                "position": 1,
                "track-count": 2,
                "format": "Digital Media",
                "tracks": [
                    {
                        "id": "track-1",
                        "number": "1",
                        "position": 1,
                        "title": "The End of the Beginning",
                        "length": 120000,
                        "artist-credit": [
                            {"name": "ATLUS Sound Team", "joinphrase": "", "artist": {}}
                        ],
                        "recording": {
                            "id": "rec-1",
                            "title": "始まりの終わり",
                            "length": 121000,
                            "artist-credit": [
                                {"name": "アトラスサウンドチーム", "joinphrase": "", "artist": {}}
                            ]
                        }
                    },
                    {
                        "id": "track-2",
                        "number": "2",
                        "position": 2,
                        "title": "Master Tactician",
                        "recording": {
                            "id": "rec-2",
                            "title": "謀略の主",
                            "length": 90000,
                            "artist-credit": [
                                {"name": "アトラスサウンドチーム", "joinphrase": "", "artist": {}}
                            ]
                        }
                    }
                ]
            }]
        }))
        .unwrap();

        let tracks = release_tracks(&release);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "The End of the Beginning");
        assert_eq!(tracks[0].artist, "ATLUS Sound Team");
        assert_eq!(tracks[0].duration.as_deref(), Some("02:00"));
        assert_eq!(tracks[0].id, "rec-1");
        assert_eq!(tracks[1].title, "Master Tactician");
        assert_eq!(tracks[1].artist, "アトラスサウンドチーム");
        assert_eq!(tracks[1].duration.as_deref(), Some("01:30"));
    }

    #[test]
    fn album_query_searches_release_and_alias_titles() {
        assert_eq!(
            album_search_params(None, "Metaphor ReFantazio").as_deref(),
            Some(
                "query=%28release%3A%28Metaphor+AND+ReFantazio%29+OR+alias%3A%28Metaphor+AND+ReFantazio%29%29"
            )
        );
    }

    #[test]
    fn album_query_keeps_the_artist_term_as_an_and() {
        assert_eq!(
            album_search_params(Some("Shoji Meguro"), "Metaphor").as_deref(),
            Some("query=artist%3A%28Shoji+AND+Meguro%29+AND+%28release%3A%28Metaphor%29+OR+alias%3A%28Metaphor%29%29")
        );
    }

    #[test]
    fn album_title_falls_back_to_the_release_title_for_localized_group_titles() {
        let tokens = query_tokens("Metaphor Refantazio");
        assert_eq!(
            album_title(
                "メタファー：リファンタジオ オリジナル・サウンドトラック",
                "Metaphor: ReFantazio Original Soundtrack",
                &tokens
            ),
            "Metaphor: ReFantazio Original Soundtrack"
        );
    }

    #[test]
    fn album_title_keeps_the_group_title_when_it_matches_the_query() {
        let tokens = query_tokens("Metaphor Refantazio");
        assert_eq!(
            album_title(
                "Metaphor: ReFantazio Special Soundtrack",
                "Metaphor: ReFantazio Special Soundtrack",
                &tokens
            ),
            "Metaphor: ReFantazio Special Soundtrack"
        );
    }

    fn album(id: &str, title: &str, date: Option<&str>) -> Album {
        Album {
            id: id.to_string(),
            title: title.to_string(),
            artist: "アトラスサウンドチーム".to_string(),
            release_date: date.map(str::to_string),
            mbid: Some(id.to_string()),
            cover_url: None,
            country: None,
            media: None,
            track_count: None,
        }
    }

    #[test]
    fn localized_editions_sort_after_titles_that_match_the_query() {
        let mut editions = vec![
            album(
                "jp",
                "メタファー：リファンタジオ オリジナル・サウンドトラック",
                Some("2025-01-15"),
            ),
            album(
                "en",
                "Metaphor: ReFantazio Original Soundtrack",
                Some("2025-01-15"),
            ),
        ];
        sort_editions(&mut editions, &query_tokens("Metaphor Refantazio"));
        assert_eq!(editions[0].id, "en");
    }

    #[test]
    fn editions_of_matching_titles_sort_by_date() {
        let mut editions = vec![
            album(
                "later",
                "Metaphor: ReFantazio Original Soundtrack",
                Some("2025-06-01"),
            ),
            album(
                "earlier",
                "Metaphor: ReFantazio Original Soundtrack",
                Some("2025-01-15"),
            ),
        ];
        sort_editions(&mut editions, &query_tokens("Metaphor Refantazio"));
        assert_eq!(editions[0].id, "earlier");
    }

    #[test]
    fn no_filters_keep_every_release() {
        let release = release("r", "Album", "2024", "Official", &["CD", "CD"]);
        let group = release_group("Album", &["Soundtrack"]);
        assert!(release_matches(&release, &group, &SearchFilters::default()));
    }

    #[test]
    fn release_type_filter_rejects_other_types() {
        let release = release("r", "Album", "2024", "Official", &["CD"]);
        let group = release_group("Single", &[]);
        let filters = SearchFilters {
            release_types: vec!["Album".to_string(), "EP".to_string()],
            ..Default::default()
        };
        assert!(!release_matches(&release, &group, &filters));
    }

    #[test]
    fn secondary_type_filter_matches_soundtracks() {
        let release = release("r", "Album", "2024", "Official", &["CD"]);
        let group = release_group("Album", &["Soundtrack"]);
        let filters = SearchFilters {
            secondary_types: vec!["Soundtrack".to_string()],
            ..Default::default()
        };
        assert!(release_matches(&release, &group, &filters));
    }

    #[test]
    fn type_and_secondary_selections_combine_as_a_union() {
        let release = release("r", "Single", "2024", "Official", &["CD"]);
        let group = release_group("Single", &[]);
        let filters = SearchFilters {
            release_types: vec!["Album".to_string(), "Single".to_string()],
            secondary_types: vec!["Soundtrack".to_string()],
            ..Default::default()
        };
        assert!(release_matches(&release, &group, &filters));
    }

    #[test]
    fn secondary_only_filter_rejects_groups_without_secondaries() {
        let release = release("r", "Album", "2024", "Official", &["CD"]);
        let group = release_group("Album", &[]);
        let filters = SearchFilters {
            secondary_types: vec!["Soundtrack".to_string()],
            ..Default::default()
        };
        assert!(!release_matches(&release, &group, &filters));
    }

    #[test]
    fn media_filter_accepts_multi_disc_releases() {
        let release = release("r", "Album", "2024", "Official", &["CD", "CD", "CD"]);
        let group = release_group("Album", &[]);
        let filters = SearchFilters {
            media_formats: vec!["CD".to_string()],
            ..Default::default()
        };
        assert!(release_matches(&release, &group, &filters));
    }

    #[test]
    fn media_filter_rejects_other_formats() {
        let release = release("r", "Album", "2024", "Official", &["Digital Media"]);
        let group = release_group("Album", &[]);
        let filters = SearchFilters {
            media_formats: vec!["CD".to_string()],
            ..Default::default()
        };
        assert!(!release_matches(&release, &group, &filters));
    }

    #[test]
    fn filter_options_are_offered_for_musicbrainz() {
        let options = filter_options();
        assert!(options.is_supported());
        assert!(options.release_types.contains(&"Album".to_string()));
        assert!(options.media_formats.contains(&"CD".to_string()));
    }

    #[test]
    fn album_search_quotes_titles_with_reserved_characters() {
        assert_eq!(
            album_search_params(Some("Ed Sheeran"), "+").as_deref(),
            Some("query=artist%3A%28Ed+AND+Sheeran%29+AND+%28release%3A%28%22%2B%22%29+OR+alias%3A%28%22%2B%22%29%29")
        );
    }

    #[test]
    fn album_search_without_query_text_lists_the_artist() {
        assert_eq!(
            album_search_params(Some("Ed Sheeran"), "  ").as_deref(),
            Some("query=artist%3A%28Ed+AND+Sheeran%29")
        );
    }

    #[test]
    fn track_search_without_query_text_lists_the_artist() {
        assert_eq!(
            track_search_params(Some("Ed Sheeran"), "").as_deref(),
            Some("query=artist%3A%28Ed+AND+Sheeran%29")
        );
    }

    #[test]
    fn search_escapes_quotes_and_backslashes() {
        assert_eq!(
            track_search_params(None, "Guns N' \"Rose\" \\ Co").as_deref(),
            Some("query=recording%3A%28Guns+AND+%22N%27%22+AND+%22%5C%22Rose%5C%22%22+AND+Co%29")
        );
    }

    #[test]
    fn search_without_artist_or_query_has_no_params() {
        assert!(album_search_params(None, "").is_none());
        assert!(track_search_params(Some(" "), "").is_none());
    }
}
