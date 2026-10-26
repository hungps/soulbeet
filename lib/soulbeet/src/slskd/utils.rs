use regex::Regex;
use shared::slskd::MatchResult;
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::LazyLock;

static RE_LEAD_TRACK_FIXED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d{1,3}|[A-D]\d{1,2})\s*[\.\-]\s*").unwrap());
static RE_TRAIL_BRACKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\[\s*[^\]]*\]\s*$").unwrap());
static RE_TRAIL_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*[-\(\[]?\d{4}[-\)\]]?\s*$").unwrap());
static RE_TRAIL_TRACK_NO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+[-\u{2013}]\s+\d{1,3}\s*$").unwrap());
static RE_MEDIUM_FOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s*(?:(?:cd|disc|disk|dvd|sacd|blu-?ray|vinyl|lp|side|medium|vol(?:ume)?)\s*[-_.]?\s*[a-z0-9]{0,3}|digital\s+media)\s*\.?\s*$",
    )
    .unwrap()
});

/// Punctuation that separates words rather than naming the release. Everything
/// outside this set that is not alphanumeric is kept as its own token, so titles
/// made of symbols (`+`, `÷`, `×`) stay comparable instead of vanishing.
const SEPARATORS: &[char] = &[
    '-', '_', '/', '\\', '.', ',', ':', ';', '\'', '"', '`', '(', ')', '[', ']', '{', '}', '!', '?',
    '|', '*', '\u{2013}', '\u{2014}', '\u{2019}', '\u{201c}', '\u{201d}',
];

fn push_word(word: &mut String, tokens: &mut HashSet<String>) {
    if !word.is_empty() {
        tokens.insert(std::mem::take(word));
    }
}

fn tokens(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
            continue;
        }
        push_word(&mut word, &mut out);
        if !c.is_whitespace() && !SEPARATORS.contains(&c) {
            out.insert(c.to_lowercase().to_string());
        }
    }
    push_word(&mut word, &mut out);
    out
}

// A struct to hold pre-processed text for efficient comparisons.
#[derive(Debug, Clone)]
struct CleanedText {
    original: String,
    words: HashSet<String>,
}

impl CleanedText {
    fn new(s: &str) -> Self {
        CleanedText {
            original: s.to_string(),
            words: tokens(s),
        }
    }

    pub fn words(&self) -> &HashSet<String> {
        &self.words
    }
}

fn jaccard_sim(a: &CleanedText, b: &CleanedText) -> f64 {
    let inter = a.words().intersection(b.words()).count();
    let uni = a.words().len() + b.words().len() - inter;
    if uni == 0 {
        0.0
    } else {
        inter as f64 / uni as f64
    }
}

fn containment_sim(candidate: &CleanedText, target: &CleanedText) -> f64 {
    if target.words().is_empty() {
        return 0.0;
    }
    let inter = candidate.words().intersection(target.words()).count();
    inter as f64 / target.words().len() as f64
}

fn dice_sim(a: &CleanedText, b: &CleanedText) -> f64 {
    let inter = a.words().intersection(b.words()).count();
    let total = a.words().len() + b.words().len();
    if total == 0 {
        1.0
    } else {
        (2.0 * inter as f64) / total as f64
    }
}

fn clean_name(name: &str) -> String {
    let name = name.replace('_', " ");
    let mut cleaned = RE_LEAD_TRACK_FIXED.replace(&name, "").to_string();
    cleaned = RE_TRAIL_BRACKET.replace(&cleaned, "").to_string();
    cleaned = RE_TRAIL_YEAR.replace(&cleaned, "").to_string();
    cleaned.trim().to_string()
}

fn extract_track_title(stem: &str) -> String {
    let stem_clean = clean_name(stem);
    if let Some(pos) = stem_clean.rfind(" - ") {
        stem_clean[pos + 3..].trim().to_string()
    } else {
        stem_clean
    }
}

fn identity_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn path_names_artist(filename: &str, artist: &str) -> bool {
    let artist = identity_text(artist);
    if artist.len() < 3 {
        return true;
    }
    identity_text(filename).contains(&artist)
}

/// The words that mark which version of a release a folder holds. `edition` and
/// `version` are left out: they describe the marker next to them ("Deluxe
/// Edition") rather than being one, and counting them makes "Deluxe" and
/// "Deluxe Edition" look like different releases.
const EDITION_WORDS: &[&str] = &[
    "deluxe",
    "expanded",
    "remaster",
    "anniversary",
    "special",
    "bonus",
    "complete",
    "reissue",
    "mono",
    "stereo",
    "acoustic",
    "instrumental",
    "live",
    "demo",
    "explicit",
    "clean",
    "international",
    "japanese",
];

fn canonical_edition_word(word: &str) -> Option<&'static str> {
    match word {
        "remastered" | "remasters" => Some("remaster"),
        other => EDITION_WORDS.iter().copied().find(|known| *known == other),
    }
}

pub fn edition_qualifiers(text: &str) -> BTreeSet<&'static str> {
    tokens(text)
        .iter()
        .filter_map(|word| canonical_edition_word(word))
        .collect()
}

/// How well a candidate folder names the same version of the release as
/// `target_album`: 1.0 for the same markers (or none on either side), 0.5 when
/// the folder is a plainer version, 0.4 when it adds markers the release does
/// not have, 0.1 when both name different ones.
pub fn edition_fit(target_album: &str, folder_name: &str) -> f64 {
    let target = edition_qualifiers(target_album);
    let candidate = edition_qualifiers(folder_name);
    if target == candidate {
        return 1.0;
    }
    if target.is_subset(&candidate) {
        return 0.4;
    }
    if candidate.is_subset(&target) {
        return 0.5;
    }
    0.1
}

/// The directory a peer's file sits in, normalized to forward slashes.
pub fn folder_of(filename: &str) -> String {
    let normalized = filename.replace('\\', "/");
    Path::new(&normalized)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// The last segment of a folder path, which is the name the peer gave the
/// release.
pub fn folder_title(folder: &str) -> String {
    let normalized = folder.replace('\\', "/");
    Path::new(&normalized)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn is_medium_folder(name: &str) -> bool {
    RE_MEDIUM_FOLDER.is_match(name)
}

pub fn release_folder(folder: &str) -> String {
    let normalized = folder.replace('\\', "/");
    let mut current = Path::new(&normalized);
    while current
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_medium_folder)
    {
        let Some(parent) = current
            .parent()
            .filter(|parent| parent.file_name().is_some())
        else {
            break;
        };
        current = parent;
    }
    current.to_string_lossy().to_string()
}

/// Whether the folder holding a file names a release of its own. A folder that
/// only repeats the artist says nothing about which release it holds; anything
/// beyond that is the peer naming one, so a folder naming a different release
/// must not have the album comparison skipped.
fn folder_claims_release(folders: &[CleanedText], artist: Option<&CleanedText>) -> bool {
    let Some(folder) = folders.last() else {
        return false;
    };
    match artist {
        Some(artist) => !folder.words().is_subset(artist.words()),
        None => !folder.words().is_empty(),
    }
}

#[derive(Debug)]
struct PathInfo {
    parent_folders: Vec<String>,
    stem: String,
}

impl PathInfo {
    fn from_path(path_str: &str) -> Self {
        let normalized_path_str = path_str.replace('\\', "/");
        let path = Path::new(&normalized_path_str);

        let parent_folders = path
            .ancestors()
            .skip(1)
            .filter_map(|p| p.file_name())
            .filter_map(|s| s.to_str())
            .map(|s| s.to_string())
            .collect::<Vec<_>>();

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let mut reversed_folders = parent_folders;
        reversed_folders.reverse();

        Self {
            parent_folders: reversed_folders,
            stem,
        }
    }
}

fn score_album(folders: &[CleanedText], target_album: &CleanedText) -> (f64, CleanedText) {
    folders
        .iter()
        .map(|folder| {
            let score =
                (jaccard_sim(folder, target_album) + containment_sim(folder, target_album)) / 2.0;
            (score, folder.clone())
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or((0.0, CleanedText::new("")))
}

fn score_artist(
    folders: &[CleanedText],
    stem: &CleanedText,
    target_artist: &CleanedText,
) -> (f64, CleanedText) {
    let folder_candidate = folders
        .iter()
        .map(|folder| (containment_sim(folder, target_artist), folder.clone()))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or((0.0, CleanedText::new("")));

    let stem_artist_part = if let Some(pos) = stem.original.rfind(" - ") {
        clean_name(&stem.original[..pos])
    } else {
        clean_name(&stem.original)
    };
    let stem_artist_part = RE_TRAIL_TRACK_NO.replace(&stem_artist_part, "").to_string();
    let stem_candidate_c = CleanedText::new(&stem_artist_part);
    let stem_score = containment_sim(&stem_candidate_c, target_artist);

    if stem_score > folder_candidate.0 {
        return (stem_score, stem_candidate_c);
    }
    if folder_candidate.0 > stem_score {
        return folder_candidate;
    }

    if stem_score > 0.9 && stem_candidate_c.original.len() > folder_candidate.1.original.len() {
        return (stem_score, stem_candidate_c);
    }
    folder_candidate
}

fn score_track(stem: &CleanedText, expected_tracks: &[CleanedText]) -> (f64, CleanedText) {
    if expected_tracks.is_empty() {
        return (1.0, CleanedText::new(&extract_track_title(&stem.original)));
    }

    let track_title_from_stem = CleanedText::new(&extract_track_title(&stem.original));

    expected_tracks
        .iter()
        .map(|expected| {
            let score = (dice_sim(&track_title_from_stem, expected) * 0.6)
                + (containment_sim(&track_title_from_stem, expected) * 0.4);
            (score, expected.clone())
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or((0.0, CleanedText::new("")))
}

pub fn rank_match(
    filename: &str,
    searched_artist: Option<&str>,
    searched_album: Option<&str>,
    expected_tracks: &[&str],
) -> MatchResult {
    const ALBUM_WEIGHT: f64 = 0.4;
    const TRACK_WEIGHT: f64 = 0.4;
    const ARTIST_WEIGHT: f64 = 0.2;
    // If a dimension scores below this, we assume the path has no useful info
    // for it (e.g. peers who do not organise by artist) and we do not let it
    // drag the total score down.
    const ALBUM_INFO_THRESHOLD: f64 = 0.25;
    const ARTIST_INFO_THRESHOLD: f64 = 0.25;

    let path_info = PathInfo::from_path(filename);
    let path_folders_c: Vec<_> = path_info
        .parent_folders
        .iter()
        .map(|f| CleanedText::new(&clean_name(f)))
        .collect();
    let stem_c = CleanedText::new(&path_info.stem);

    let searched_artist_c = searched_artist.map(CleanedText::new);
    let (artist_score, best_artist_guess) = match searched_artist_c.as_ref() {
        Some(target) => score_artist(&path_folders_c, &stem_c, target),
        None => (0.0, CleanedText::new("")),
    };

    let (album_score, best_album_folder) = if let Some(album_str) = searched_album {
        let searched_album_c = CleanedText::new(album_str);
        score_album(&path_folders_c, &searched_album_c)
    } else {
        (0.0, CleanedText::new(""))
    };

    let (track_score, best_track_match) = if !expected_tracks.is_empty() {
        let expected_tracks_c: Vec<_> = expected_tracks
            .iter()
            .map(|t| CleanedText::new(t))
            .collect();
        score_track(&stem_c, &expected_tracks_c)
    } else {
        (
            0.0,
            CleanedText::new(&extract_track_title(&stem_c.original)),
        )
    };

    let mut weighted_sum = 0.0;
    let mut total_weight = 0.0;

    if searched_artist.is_some() {
        weighted_sum += artist_score * ARTIST_WEIGHT;
        if artist_score > ARTIST_INFO_THRESHOLD {
            total_weight += ARTIST_WEIGHT;
        }
    }

    if !expected_tracks.is_empty() {
        weighted_sum += track_score * TRACK_WEIGHT;
        total_weight += TRACK_WEIGHT;
    }

    if searched_album.is_some() {
        weighted_sum += album_score * ALBUM_WEIGHT;
        if album_score > ALBUM_INFO_THRESHOLD
            || folder_claims_release(&path_folders_c, searched_artist_c.as_ref())
        {
            total_weight += ALBUM_WEIGHT;
        }
    }

    let total_score = if total_weight > 0.0 {
        weighted_sum / total_weight
    } else {
        0.0
    };

    let guessed_album = match searched_album {
        Some(album) if album_score <= ALBUM_INFO_THRESHOLD => album.to_string(),
        _ => best_album_folder.original,
    };

    MatchResult {
        guessed_artist: best_artist_guess.original,
        guessed_album,
        matched_track: best_track_match.original,
        artist_score,
        album_score,
        track_score,
        total_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album_score(folder: &str, album: &str) -> f64 {
        rank_match(
            &format!("/music/{folder}/01 - Someone Like You.flac"),
            Some("Ed Sheeran"),
            Some(album),
            &["Someone Like You"],
        )
        .album_score
    }

    #[test]
    fn a_medium_folder_does_not_become_the_guessed_album() {
        let result = rank_match(
            "/music/Artist/CD1/01 - Song.flac",
            Some("Artist"),
            Some("Album"),
            &["Song"],
        );
        assert_eq!(result.guessed_album, "Album");
    }

    #[test]
    fn a_folder_naming_the_release_is_kept_as_the_guessed_album() {
        let result = rank_match(
            "/music/Artist/Album (Deluxe)/CD1/01 - Song.flac",
            Some("Artist"),
            Some("Album"),
            &["Song"],
        );
        assert_eq!(result.guessed_album, "Album (Deluxe)");
    }

    #[test]
    fn symbol_titles_are_compared() {
        assert!(album_score("Ed Sheeran - +", "+") > 0.6);
        assert!(album_score("Ed Sheeran - + (Deluxe)", "+") > 0.6);
        assert!(album_score("Ed Sheeran - x", "+") < 0.25);
    }

    #[test]
    fn symbols_survive_the_tokenizer() {
        assert_eq!(
            tokens("Ed Sheeran - + (Deluxe)"),
            ["ed", "sheeran", "+", "deluxe"]
                .map(str::to_string)
                .into_iter()
                .collect()
        );
        assert!(tokens("÷").contains("÷"));
        assert!(tokens("Metaphor: ReFantazio").contains("refantazio"));
        assert!(!tokens("AC/DC").contains("/"));
    }

    #[test]
    fn edition_markers_read_the_same_words() {
        assert_eq!(
            edition_qualifiers("Divide (Deluxe Edition)"),
            edition_qualifiers("Ed Sheeran - Divide [Deluxe]")
        );
        assert!(edition_qualifiers("Divide").is_empty());
        assert_eq!(
            edition_qualifiers("Album (2011 Remastered Version)"),
            ["remaster"].into_iter().collect()
        );
    }

    #[test]
    fn edition_fit_ranks_the_same_version_first() {
        assert_eq!(edition_fit("÷ (Deluxe)", "Ed Sheeran - ÷ (Deluxe)"), 1.0);
        assert_eq!(edition_fit("÷", "Ed Sheeran - ÷"), 1.0);
        assert_eq!(edition_fit("÷ (Deluxe)", "Ed Sheeran - ÷"), 0.5);
        assert_eq!(edition_fit("÷", "Ed Sheeran - ÷ [Deluxe Edition]"), 0.4);
        assert_eq!(edition_fit("÷ (Deluxe)", "Ed Sheeran - ÷ (Live)"), 0.1);
    }

    fn total_score(folder: &str) -> f64 {
        rank_match(
            &format!("/music/{folder}/01 - Afterglow.flac"),
            Some("Ed Sheeran"),
            Some("Afterglow"),
            &["Afterglow"],
        )
        .total_score
    }

    #[test]
    fn a_folder_naming_the_release_beats_one_naming_another() {
        assert!(
            total_score("Ed Sheeran - Afterglow")
                > total_score("Ed Sheeran - +-=÷× (TOUR COLLECTION) (2024)")
        );
    }

    #[test]
    fn a_folder_that_only_repeats_the_artist_is_not_dragged_down() {
        assert_eq!(total_score("Ed Sheeran"), 1.0);
    }

    #[test]
    fn folder_helpers_read_peer_paths() {
        assert_eq!(
            folder_of("Music\\Ed Sheeran - ÷ (Deluxe)\\01 - Eraser.flac"),
            "Music/Ed Sheeran - ÷ (Deluxe)"
        );
        assert_eq!(
            folder_title("Music/Ed Sheeran - ÷ (Deluxe)"),
            "Ed Sheeran - ÷ (Deluxe)"
        );
        assert_eq!(folder_of("Eraser.flac"), "");
        assert_eq!(folder_title(""), "");
    }

    #[test]
    fn release_folders_drop_medium_names() {
        assert_eq!(
            release_folder("Music/Ed Sheeran - ÷/CD1"),
            "Music/Ed Sheeran - ÷"
        );
        assert_eq!(
            release_folder("Music/Ed Sheeran - ÷/Disc 2"),
            "Music/Ed Sheeran - ÷"
        );
        assert_eq!(
            release_folder("Music/Ed Sheeran - ÷/CD1/CD2"),
            "Music/Ed Sheeran - ÷"
        );
        assert_eq!(
            release_folder("Music/Ed Sheeran - ÷"),
            "Music/Ed Sheeran - ÷"
        );
        assert_eq!(
            release_folder("Music/Ed Sheeran - ÷ [CD FLAC]"),
            "Music/Ed Sheeran - ÷ [CD FLAC]"
        );
        assert_eq!(release_folder("CD1"), "CD1");
    }
}
