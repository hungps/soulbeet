//! 30-second preview clips from Deezer's public catalogue.
//!
//! Soulseek can only hand over whole files, so a source cannot be heard before
//! it is downloaded. What a preview can tell is whether a search result is the
//! recording being looked for: Deezer serves one clip per track, keyed by
//! artist and title, free and without an API key.

use std::time::Duration as StdDuration;

use serde::Deserialize;
use tracing::debug;

const ENDPOINT: &str = "https://api.deezer.com/search/track";
/// Deezer answers a search with whole tracks; only near the length asked for
/// is a candidate taken, so a live take or a radio edit is not offered as the
/// release.
const LENGTH_TOLERANCE_SECS: u32 = 3;
const CONNECT_TIMEOUT_SECS: u64 = 5;
const REQUEST_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Deserialize)]
struct Catalog {
    #[serde(default)]
    data: Vec<CatalogTrack>,
}

#[derive(Debug, Deserialize)]
struct CatalogTrack {
    pub duration: u32,
    #[serde(default)]
    pub preview: String,
}

/// The clip Deezer has for `artist` — `title`, when one sounds close to
/// `seconds` long. `Ok(None)` means the catalogue has nothing usable.
pub async fn find_clip(
    artist: &str,
    title: &str,
    seconds: Option<u32>,
) -> Result<Option<String>, String> {
    let query = [artist.trim(), title.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() {
        return Ok(None);
    }

    let client = reqwest::Client::builder()
        .connect_timeout(StdDuration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(StdDuration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(ENDPOINT)
        .query(&[("q", query.as_str()), ("limit", "10")])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Deezer answered {}", response.status()));
    }
    let catalog: Catalog = response.json().await.map_err(|e| e.to_string())?;
    let clip = pick(catalog.data, seconds);
    debug!("Deezer preview for '{query}': {clip:?}");
    Ok(clip)
}

/// The first candidate with a clip whose length is close enough to `seconds`.
/// Without a length to compare, the first candidate with a clip is taken.
fn pick(candidates: Vec<CatalogTrack>, seconds: Option<u32>) -> Option<String> {
    let usable = candidates
        .into_iter()
        .filter(|track| !track.preview.trim().is_empty());
    match seconds {
        Some(seconds) => usable
            .filter(|track| track.duration.abs_diff(seconds) <= LENGTH_TOLERANCE_SECS)
            .map(|track| track.preview)
            .next(),
        None => usable.map(|track| track.preview).next(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(seconds: u32, preview: &str) -> CatalogTrack {
        CatalogTrack {
            duration: seconds,
            preview: preview.to_string(),
        }
    }

    #[test]
    fn a_candidate_of_another_length_is_not_offered_as_the_release() {
        let candidates = vec![
            track(240, "live"),
            track(176, "studio"),
            track(176 + LENGTH_TOLERANCE_SECS + 1, "radio edit"),
        ];
        assert_eq!(pick(candidates, Some(176)).as_deref(), Some("studio"));
        assert_eq!(pick(vec![track(300, "long")], Some(176)), None);
    }

    #[test]
    fn without_a_length_the_first_clip_is_taken() {
        let candidates = vec![track(0, ""), track(300, "long"), track(200, "other")];
        assert_eq!(pick(candidates, None).as_deref(), Some("long"));
        assert_eq!(pick(Vec::new(), None), None);
    }
}
