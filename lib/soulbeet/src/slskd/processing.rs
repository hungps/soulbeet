use super::utils;
use crate::audio::is_audio_path;
use crate::slskd::models::SearchResponse;
use itertools::Itertools;
use shared::slskd::{AlbumResult, MatchResult, SearchResult, TrackResult};
use std::collections::HashMap;
use std::path::Path;

pub fn process_search_responses(
    responses: &[SearchResponse],
    searched_artist: &str,
    searched_album: Option<&str>,
    expected_tracks: &[&str],
    require_artist_match: bool,
) -> Vec<AlbumResult> {
    const MIN_SCORE_THRESHOLD: f64 = 0.6;
    let folder_files = count_folder_files(responses);
    let scored_files: Vec<(MatchResult, SearchResult)> = responses
        .iter()
        .flat_map(|resp| {
            resp.files.iter().filter_map(|file| {
                let path = Path::new(&file.filename);
                if path.extension().is_some() && !is_audio_path(path) {
                    return None;
                }

                if require_artist_match
                    && !utils::path_names_artist(&file.filename, searched_artist)
                {
                    return None;
                }

                let rank_result = utils::rank_match(
                    &file.filename,
                    Some(searched_artist),
                    searched_album,
                    expected_tracks,
                );

                if rank_result.total_score < MIN_SCORE_THRESHOLD {
                    return None;
                }

                let search_result = SearchResult {
                    username: resp.username.clone(),
                    filename: file.filename.clone(),
                    size: file.size,
                    bitrate: file.bit_rate,
                    duration: file.length,
                    sample_rate: file.sample_rate,
                    bit_depth: file.bit_depth,
                    has_free_upload_slot: resp.has_free_upload_slot,
                    upload_speed: resp.upload_speed,
                    queue_length: resp.queue_length,
                };
                Some((rank_result, search_result))
            })
        })
        .collect();

    find_best_albums(
        &scored_files,
        expected_tracks,
        searched_album,
        &folder_files,
    )
}

/// Audio files per (peer, folder), counted before any matching so a folder's
/// size is known even when most of it matches no expected track.
fn count_folder_files(responses: &[SearchResponse]) -> HashMap<(String, String), usize> {
    let mut counts = HashMap::new();
    for response in responses {
        for file in &response.files {
            let path = Path::new(&file.filename);
            if path.extension().is_some() && !is_audio_path(path) {
                continue;
            }
            *counts
                .entry((
                    response.username.clone(),
                    utils::release_folder(&utils::folder_of(&file.filename)),
                ))
                .or_insert(0) += 1;
        }
    }
    counts
}

/// How close a folder's audio file count is to the release's track count. A
/// couple of extra or missing files (scans, a bonus track) are normal; every
/// further one weakens the fit. Neutral only when the search is not for a
/// release, where the folder legitimately holds more tracks than were asked
/// for.
fn count_fit(files_in_folder: usize, expected: Option<usize>) -> f64 {
    const SLACK: usize = 2;
    let Some(expected) = expected.filter(|expected| *expected > 0) else {
        return 1.0;
    };
    let off_by = files_in_folder.abs_diff(expected).saturating_sub(SLACK);
    (1.0 - off_by as f64 / expected as f64).clamp(0.0, 1.0)
}

fn find_best_albums(
    scored_files: &[(MatchResult, SearchResult)],
    expected_tracks: &[&str],
    searched_album: Option<&str>,
    folder_files: &HashMap<(String, String), usize>,
) -> Vec<AlbumResult> {
    if expected_tracks.is_empty() {
        return vec![];
    }

    let album_groups = scored_files.iter().into_group_map_by(|(_, search)| {
        (
            search.username.clone(),
            utils::release_folder(&utils::folder_of(&search.filename)),
        )
    });

    album_groups
        .into_iter()
        .filter_map(|((username, folder), files_in_group)| {
            let artist = files_in_group
                .first()
                .map(|(rank, _)| rank.guessed_artist.clone())
                .unwrap_or_default();
            // Specific search: find the single best file for each expected track.
            let mut best_files_for_album = HashMap::new();

            for expected_track_title in expected_tracks {
                if let Some(best_file_for_track) = files_in_group
                    .iter()
                    // Find all files that matched this specific track
                    .filter(|(rank, _)| &rank.matched_track == expected_track_title)
                    // Find the best one among them
                    .max_by(|(r1, s1), (r2, s2)| {
                        r1.total_score
                            .partial_cmp(&r2.total_score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| {
                                s1.quality_score().partial_cmp(&s2.quality_score()).unwrap()
                            })
                            .then_with(|| {
                                s1.availability_score()
                                    .partial_cmp(&s2.availability_score())
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                    })
                {
                    best_files_for_album.insert(*expected_track_title, best_file_for_track);
                }
            }

            let final_tracks: Vec<_> = expected_tracks
                .iter()
                .filter_map(|t| best_files_for_album.get(*t))
                .map(|(mr, sr)| TrackResult::new(sr.clone(), mr.clone()))
                .collect();

            if final_tracks.is_empty() {
                return None;
            }

            let completeness = if !expected_tracks.is_empty() {
                final_tracks.len() as f64 / expected_tracks.len() as f64
            } else {
                1.0
            };

            let total_size: i64 = final_tracks.iter().map(|t| t.base.size).sum();
            let dominant_quality = final_tracks
                .iter()
                .map(|t| t.base.quality())
                .counts()
                .into_iter()
                .max_by_key(|&(_, count)| count)
                .map(|(val, _)| val)
                .unwrap_or_default();

            let first_track = final_tracks[0].base.clone();
            let album_title = utils::folder_title(&folder);
            let file_count = folder_files
                .get(&(username.clone(), folder.clone()))
                .copied()
                .unwrap_or(final_tracks.len())
                .max(final_tracks.len());

            let avg_score: f64 =
                final_tracks.iter().map(|t| t.match_score).sum::<f64>() / final_tracks.len() as f64;
            let avg_format_score = final_tracks
                .iter()
                .map(|t| t.base.quality_score())
                .sum::<f64>()
                / final_tracks.len() as f64;
            let avg_availability = final_tracks
                .iter()
                .map(|t| t.base.availability_score())
                .sum::<f64>()
                / final_tracks.len() as f64;

            let edition_fit = searched_album
                .map(|album| utils::edition_fit(album, &album_title))
                .unwrap_or(1.0);
            let expected_count = searched_album.map(|_| expected_tracks.len());
            let count_fit = count_fit(file_count, expected_count);

            let album_quality_score = (avg_score * 0.25)
                + (completeness * 0.25)
                + (edition_fit * 0.20)
                + (count_fit * 0.10)
                + (avg_format_score * 0.18)
                + (avg_availability * 0.02);

            Some(AlbumResult {
                username,
                album_path: folder,
                album_title,
                artist: Some(artist),
                track_count: final_tracks.len(),
                total_size,
                tracks: final_tracks,
                dominant_quality,
                has_free_upload_slot: first_track.has_free_upload_slot,
                upload_speed: first_track.upload_speed,
                queue_length: first_track.queue_length,
                availability_score: avg_availability,
                file_count,
                release_fit: edition_fit.min(count_fit),
                score: album_quality_score,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slskd::models::{SearchResponse, SearchResponseFile};

    fn response(username: &str, folder: &str, tracks: &[&str]) -> SearchResponse {
        SearchResponse {
            username: username.to_string(),
            files: tracks
                .iter()
                .enumerate()
                .map(|(index, title)| SearchResponseFile {
                    filename: format!("/music/{folder}/{:02} - {title}.flac", index + 1),
                    size: 30_000_000,
                    bit_rate: None,
                    length: Some(240),
                    sample_rate: Some(44100),
                    bit_depth: Some(16),
                })
                .collect(),
            has_free_upload_slot: true,
            upload_speed: 1_000_000,
            queue_length: 0,
        }
    }

    const PLAIN_TRACKS: [&str; 8] = [
        "Eraser", "Castle", "Dive", "Shape", "Perfect", "Galway", "Happier", "Hearts",
    ];
    const BONUS_TRACKS: [&str; 4] = ["Supermarket", "Butterflies", "Sandman", "Remedy"];

    fn deluxe_tracks() -> Vec<&'static str> {
        PLAIN_TRACKS
            .into_iter()
            .chain(BONUS_TRACKS)
            .collect()
    }

    fn ranked(responses: &[SearchResponse], album: &str, expected: &[&str]) -> Vec<AlbumResult> {
        let mut albums =
            process_search_responses(responses, "Ed Sheeran", Some(album), expected, false);
        albums.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        albums
    }

    #[test]
    fn a_deluxe_release_ranks_the_deluxe_folder_first() {
        let deluxe = deluxe_tracks();
        let responses = vec![
            response("peer", "Ed Sheeran - ÷", &PLAIN_TRACKS),
            response("peer", "Ed Sheeran - ÷ (Deluxe)", &deluxe),
        ];

        let albums = ranked(&responses, "÷ (Deluxe)", &deluxe);

        assert_eq!(albums.len(), 2);
        let best = &albums[0];
        assert_eq!(best.album_title, "Ed Sheeran - ÷ (Deluxe)");
        assert_eq!(best.album_path, "/music/Ed Sheeran - ÷ (Deluxe)");
        assert_eq!(best.track_count, deluxe.len());
        assert_eq!(best.file_count, deluxe.len());
        assert_eq!(best.release_fit, 1.0);
        // The plainer folder still lists, it just ranks lower.
        assert_eq!(albums[1].album_title, "Ed Sheeran - ÷");
        assert!(albums[1].score < best.score);
    }

    #[test]
    fn a_plain_release_ranks_the_deluxe_folder_last() {
        let deluxe = deluxe_tracks();
        let responses = vec![
            response("peer", "Ed Sheeran - ÷ (Deluxe)", &deluxe),
            response("peer", "Ed Sheeran - ÷", &PLAIN_TRACKS),
        ];

        let albums = ranked(&responses, "÷", &PLAIN_TRACKS);

        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].album_title, "Ed Sheeran - ÷");
        assert_eq!(albums[0].release_fit, 1.0);
        // Its extra files are past the slack, and it names another edition.
        assert_eq!(albums[1].album_title, "Ed Sheeran - ÷ (Deluxe)");
        assert!(albums[1].release_fit < 0.5);
    }

    #[test]
    fn symbol_titled_releases_still_prefer_their_own_version() {
        let deluxe = deluxe_tracks();
        let responses = vec![
            response("peer", "Ed Sheeran - +", &PLAIN_TRACKS),
            response("peer", "Ed Sheeran - + (Deluxe)", &deluxe),
        ];

        assert_eq!(
            ranked(&responses, "+", &PLAIN_TRACKS)[0].album_title,
            "Ed Sheeran - +"
        );
        assert_eq!(
            ranked(&responses, "+ (Deluxe)", &deluxe)[0].album_title,
            "Ed Sheeran - + (Deluxe)"
        );
    }

    #[test]
    fn a_track_search_is_not_held_against_a_full_album_folder() {
        let responses = vec![response("peer", "Ed Sheeran - ÷", &PLAIN_TRACKS)];

        let albums = process_search_responses(&responses, "Ed Sheeran", None, &["Dive"], false);

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].file_count, PLAIN_TRACKS.len());
        assert_eq!(albums[0].release_fit, 1.0);
    }

    #[test]
    fn two_peers_sharing_a_folder_name_stay_apart() {
        let responses = vec![
            response("peer-a", "Ed Sheeran - ÷", &PLAIN_TRACKS),
            response("peer-b", "Ed Sheeran - ÷", &PLAIN_TRACKS),
        ];

        let albums = ranked(&responses, "÷", &PLAIN_TRACKS);

        assert_eq!(albums.len(), 2);
        assert_ne!(albums[0].username, albums[1].username);
    }

    #[test]
    fn medium_folders_merge_into_their_release() {
        let responses = vec![
            response("peer", "Ed Sheeran - ÷/CD1", &PLAIN_TRACKS[..4]),
            response("peer", "Ed Sheeran - ÷/CD2", &PLAIN_TRACKS[4..]),
        ];

        let albums = ranked(&responses, "÷", &PLAIN_TRACKS);

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].album_title, "Ed Sheeran - ÷");
        assert_eq!(albums[0].album_path, "/music/Ed Sheeran - ÷");
        assert_eq!(albums[0].track_count, PLAIN_TRACKS.len());
        assert_eq!(albums[0].file_count, PLAIN_TRACKS.len());
    }

    #[test]
    fn artist_prefixed_track_numbers_do_not_split_the_album() {
        let titles = [
            "The King's Trial",
            "Unfamiliar Lands",
            "A Dying World",
            "Beyond the Horizon",
            "Final Conflict",
        ];
        let response = SearchResponse {
            username: "peer".to_string(),
            files: titles
                .iter()
                .enumerate()
                .map(|(index, title)| SearchResponseFile {
                    filename: format!(
                        "/music/Metaphor ReFantazio OST/ATLUS Sound Team - {:02} - {title}.flac",
                        index + 1
                    ),
                    size: 30_000_000,
                    bit_rate: None,
                    length: Some(240),
                    sample_rate: Some(44100),
                    bit_depth: Some(16),
                })
                .collect(),
            has_free_upload_slot: true,
            upload_speed: 1_000_000,
            queue_length: 0,
        };

        let albums = process_search_responses(
            &[response],
            "ATLUS Sound Team",
            Some("Metaphor: ReFantazio Original Soundtrack"),
            &titles,
            false,
        );

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].track_count, titles.len());
    }

    #[test]
    fn title_only_results_must_name_the_artist() {
        let titles = ["The King's Trial", "Unfamiliar Lands", "A Dying World"];
        let responses = vec![response("peer", "Metaphor ReFantazio OST", &titles)];

        let loose = process_search_responses(
            &responses,
            "ATLUS Sound Team",
            Some("Metaphor: ReFantazio Original Soundtrack"),
            &titles,
            false,
        );
        assert_eq!(loose.len(), 1);

        let strict = process_search_responses(
            &responses,
            "ATLUS Sound Team",
            Some("Metaphor: ReFantazio Original Soundtrack"),
            &titles,
            true,
        );
        assert!(strict.is_empty());
    }

    #[test]
    fn soundtrack_tracks_from_a_plain_ost_folder_are_kept() {
        let titles = [
            "The King's Trial",
            "Unfamiliar Lands",
            "A Dying World",
            "Beyond the Horizon",
            "Final Conflict",
        ];
        let responses = vec![response("peer", "Metaphor ReFantazio OST", &titles)];

        let albums = process_search_responses(
            &responses,
            "ATLUS Sound Team",
            Some("Metaphor: ReFantazio Original Soundtrack"),
            &titles,
            false,
        );

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].track_count, titles.len());
    }

    #[test]
    fn a_single_track_release_prefers_the_folder_named_after_it() {
        let tour = ["Afterglow", "Perfect", "Shape of You", "Castle", "Dive"];
        let responses = vec![
            response(
                "tour-peer",
                "Ed Sheeran - +-=÷× (TOUR COLLECTION) (2024)",
                &tour,
            ),
            response("single-peer", "Ed Sheeran - Afterglow", &["Afterglow"]),
        ];

        let albums = ranked(&responses, "Afterglow", &["Afterglow"]);

        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].album_title, "Ed Sheeran - Afterglow");
        assert_eq!(albums[0].release_fit, 1.0);
        // The tour folder still lists, but its five files are no fit for a
        // one-track release and it names another release than the one searched.
        assert_eq!(
            albums[1].album_title,
            "Ed Sheeran - +-=÷× (TOUR COLLECTION) (2024)"
        );
        assert_eq!(albums[1].file_count, tour.len());
        assert!(albums[1].release_fit < 0.5);
        assert!(albums[1].score < albums[0].score);
    }
}
