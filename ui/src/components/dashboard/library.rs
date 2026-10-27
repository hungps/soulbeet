use dioxus::logger::tracing::info;
use dioxus::prelude::*;
use shared::download::DownloadQuery;
use shared::library::{
    album_activity_key, normalize_key, track_activity_key, DownloadActivity, LibraryMissingTrack,
    LibraryRelease, LibraryReleaseDetail, LibraryTagApplyRequest, LibraryTagApplyTrack,
    LibraryTagPreview, LibraryTagPreviewRequest, LibraryTagTrack, LibraryTrackInfo,
    ReleaseCompleteness,
};
use shared::metadata::{Album, Track};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::use_auth;
use crate::{
    library_audio_url, use_player, ConfirmModal, CoverImage, DownloadTick, Modal, PlayerTrack,
    QualityBadge,
};

#[derive(Clone, PartialEq)]
enum PendingRemoval {
    Release(String),
    Track(String),
}

#[derive(Clone, PartialEq)]
enum PendingRequest {
    Release(Box<LibraryRelease>),
    Track(Box<LibraryRelease>, Box<LibraryTrackInfo>),
    Missing(Box<LibraryRelease>, Box<LibraryMissingTrack>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibraryFocus {
    pub release: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

impl LibraryFocus {
    pub fn is_empty(&self) -> bool {
        self.release.is_none() && self.artist.is_none() && self.album.is_none()
    }

    fn matches(&self, release: &LibraryRelease) -> bool {
        if let Some(wanted) = self.release.as_deref() {
            if release.id == wanted || release.mbid.as_deref() == Some(wanted) {
                return true;
            }
        }
        if self.artist.is_none() && self.album.is_none() {
            return false;
        }
        let artist_matches = self
            .artist
            .as_deref()
            .is_none_or(|wanted| normalize_key(wanted) == normalize_key(&release.album_artist));
        let album_matches = self
            .album
            .as_deref()
            .is_none_or(|wanted| normalize_key(wanted) == normalize_key(&release.album));
        artist_matches && album_matches
    }
}

#[derive(Clone, Debug, PartialEq)]
enum LibraryRow {
    Track(LibraryTrackInfo),
    Missing(LibraryMissingTrack),
}

impl LibraryRow {
    fn disc_number(&self) -> Option<u32> {
        match self {
            Self::Track(track) => track.disc_number,
            Self::Missing(track) => track.disc_number,
        }
    }

    fn track_number(&self) -> Option<u32> {
        match self {
            Self::Track(track) => track.track_number,
            Self::Missing(track) => track.track_number,
        }
    }

    fn title(&self) -> &str {
        match self {
            Self::Track(track) => &track.title,
            Self::Missing(track) => &track.title,
        }
    }
}

fn ordered_library_rows(detail: &LibraryReleaseDetail) -> Vec<LibraryRow> {
    let mut rows: Vec<LibraryRow> = detail
        .tracks
        .iter()
        .cloned()
        .map(LibraryRow::Track)
        .chain(
            detail
                .missing_tracks
                .iter()
                .cloned()
                .map(LibraryRow::Missing),
        )
        .collect();
    rows.sort_by(|left, right| {
        left.disc_number()
            .unwrap_or(1)
            .cmp(&right.disc_number().unwrap_or(1))
            .then_with(|| {
                left.track_number()
                    .unwrap_or(u32::MAX)
                    .cmp(&right.track_number().unwrap_or(u32::MAX))
            })
            .then_with(|| {
                left.title()
                    .to_lowercase()
                    .cmp(&right.title().to_lowercase())
            })
    });
    rows
}

fn track_number_label(disc: Option<u32>, number: Option<u32>) -> String {
    match (disc, number) {
        (Some(disc), Some(number)) if disc > 1 => format!("{disc}.{number:02}"),
        (_, Some(number)) => format!("{number:02}"),
        _ => "—".to_string(),
    }
}

fn relative_track_path(path: &str, folder_path: &str) -> String {
    let relative = std::path::Path::new(path)
        .strip_prefix(folder_path)
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    relative.trim_start_matches(['/', '\\']).to_string()
}

fn scroll_to_release(release_id: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(element) = window
        .document()
        .and_then(|document| document.get_element_by_id(&format!("library-release-{release_id}")))
    else {
        return;
    };
    element.scroll_into_view();
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

fn completeness_label(
    release: &LibraryRelease,
    active_tracks: u32,
    has_active_request: bool,
) -> (String, &'static str) {
    let total = release
        .expected_track_count
        .map(|expected| expected.max(release.track_count))
        .unwrap_or(release.track_count);
    if release.ignored {
        return (
            format!("Ignored · {}/{}", release.matched_track_count, total),
            "bg-gray-700/20 text-gray-500 border-gray-600/40",
        );
    }
    if has_active_request {
        let active = active_tracks.min(release.matched_track_count);
        let matched = release.matched_track_count - active;
        return (
            format!("Partial · {matched}/{total}"),
            "bg-yellow-600/20 text-yellow-300 border-yellow-500/40",
        );
    }
    let matched = format!("{}/{}", release.matched_track_count, total);
    match release.completeness {
        ReleaseCompleteness::Complete => (
            format!("Complete · {matched}"),
            "bg-green-600/20 text-green-300 border-green-500/40",
        ),
        ReleaseCompleteness::Partial => (
            format!("Partial · {matched}"),
            "bg-yellow-600/20 text-yellow-300 border-yellow-500/40",
        ),
        ReleaseCompleteness::Unmatched => (
            format!("Unknown · {matched}"),
            "bg-gray-600/20 text-gray-400 border-gray-500/40",
        ),
    }
}

fn download_state_badge(state: Option<&String>) -> Element {
    let Some(state) = state else {
        return rsx! {};
    };
    let (label, class) = if state == "downloading" {
        (
            "Downloading",
            "bg-blue-600/20 text-blue-300 border-blue-500/40",
        )
    } else {
        (
            "Requested",
            "bg-orange-600/20 text-orange-300 border-orange-500/40",
        )
    };
    rsx! {
        span {
            class: "text-[9px] font-mono px-1.5 py-0.5 rounded border uppercase shrink-0 {class}",
            "{label}"
        }
    }
}

fn url_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn open_search(artist: &str, album: &str, release_mbid: Option<&str>, search_type: &str) {
    let mut url = format!(
        "?artist={}&q={}&type={}",
        url_encode(artist),
        url_encode(album),
        url_encode(search_type)
    );
    if let Some(mbid) = release_mbid.filter(|value| !value.is_empty()) {
        url.push_str(&format!("&release={}", url_encode(mbid)));
    }
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href(&format!("/{url}"));
    }
}

pub(crate) fn title_overlap(left: &str, right: &str) -> f64 {
    let left: HashSet<&str> = left.split_whitespace().collect();
    let right: HashSet<&str> = right.split_whitespace().collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    left.intersection(&right).count() as f64 / left.union(&right).count() as f64
}

pub(crate) fn suggest_track(
    file_title: &str,
    file_number: Option<u32>,
    tracks: &[LibraryTagTrack],
) -> Option<usize> {
    tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let candidate_key = normalize_key(&track.title);
            let file_key = normalize_key(file_title);
            let mut score = if candidate_key.is_empty() || file_key.is_empty() {
                0.0
            } else if candidate_key == file_key {
                1.0
            } else if candidate_key.contains(&file_key) || file_key.contains(&candidate_key) {
                0.75
            } else {
                title_overlap(&track.title.to_lowercase(), &file_title.to_lowercase()) * 0.5
            };
            if let (Some(file_number), Some(track_number)) = (file_number, track.track_number) {
                if file_number == track_number {
                    score += 0.5;
                }
            }
            (index, score)
        })
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .filter(|(_, score)| *score >= 0.5)
        .map(|(index, _)| index)
}

fn is_unknown(release: &LibraryRelease) -> bool {
    release.matched_track_count == 0 || release.completeness == ReleaseCompleteness::Unmatched
}

fn filled(value: Option<&String>) -> Option<&str> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// The album with MusicBrainz's disambiguation, so two pressings of the same
/// album no longer read alike.
fn release_title(release: &LibraryRelease) -> String {
    match filled(release.album_disambig.as_ref()) {
        Some(disambig) => format!("{} ({disambig})", release.album),
        None => release.album.clone(),
    }
}

/// Which pressing it is: `2017 · CD · GB`. The artist is not repeated, since
/// the library groups the release under it already.
fn release_subtitle(release: &LibraryRelease) -> String {
    let mut parts = Vec::new();
    if let Some(year) = release.year {
        parts.push(year.to_string());
    }
    parts.extend(filled(release.media.as_ref()).map(str::to_string));
    parts.extend(filled(release.country.as_ref()).map(str::to_string));
    parts.join(" · ")
}

/// Who the library files the release under: MusicBrainz's first credit when it
/// knows one, the scanned album artist otherwise.
fn release_artist(release: &LibraryRelease) -> String {
    filled(release.main_artist.as_ref())
        .unwrap_or(&release.album_artist)
        .to_string()
}

/// The releases by the act they belong to, so a featured credit is filed under
/// the main artist instead of a heading of its own.
fn group_by_artist(
    releases: &[LibraryRelease],
    main_artists: &HashMap<String, String>,
) -> BTreeMap<String, Vec<LibraryRelease>> {
    releases.iter().cloned().fold(BTreeMap::new(), |mut map, release| {
        let artist = main_artists
            .get(&release.id)
            .cloned()
            .unwrap_or_else(|| release_artist(&release));
        map.entry(artist).or_default().push(release);
        map
    })
}

fn track_quality_badge(track: &LibraryTrackInfo) -> Element {
    let Some(format) = track.audio_format.as_deref() else {
        return rsx! {};
    };
    rsx! {
        QualityBadge {
            quality: format.to_string(),
            bitrate: track.bitrate.map(|value| value as i32),
            bit_depth: track.bit_depth.map(|value| value as i32),
            sample_rate: track.sample_rate.map(|value| value as i32),
            compact: true,
        }
    }
}

fn format_track_duration(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[component]
fn LibraryTrackRow(
    track: LibraryTrackInfo,
    folder_path: String,
    download_state: Option<String>,
    on_tag: EventHandler,
    on_search: EventHandler,
    on_request: EventHandler,
    on_remove: EventHandler,
) -> Element {
    let player = use_player();
    let play_track = PlayerTrack {
        url: library_audio_url(&track.id),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration: track.duration,
    };
    let path = relative_track_path(&track.path, &folder_path);
    let number = track_number_label(track.disc_number, track.track_number);

    rsx! {
        div { class: "flex items-center gap-3 p-2 rounded hover:bg-white/5 group",
            button {
                class: "p-1.5 rounded-full hover:bg-white/10 text-gray-500 hover:text-beet-accent transition-all cursor-pointer shrink-0",
                title: "Preview track",
                aria_label: "Preview track",
                onclick: move |_| player.play_track(play_track.clone()),
                svg {
                    class: "w-3.5 h-3.5",
                    fill: "currentColor",
                    view_box: "0 0 24 24",
                    path { d: "M8 5v14l11-7z" }
                }
            }
            span { class: "text-[10px] font-mono text-gray-600 w-8 text-right shrink-0",
                "{number}"
            }
            div { class: "flex-1 min-w-0",
                div { class: "flex items-center gap-2 min-w-0",
                    p { class: "text-xs font-mono text-gray-300 break-words min-w-0",
                        "{track.title}"
                    }
                    {track_quality_badge(&track)}
                    {download_state_badge(download_state.as_ref())}
                }
                p { class: "text-[10px] font-mono text-gray-600 break-all mt-0.5",
                    "{path}"
                    if let Some(duration) = track.duration {
                        span { " · {format_track_duration(duration)}" }
                    }
                }
            }
            div { class: "flex items-center gap-1 shrink-0",
                if !track.matched {
                    button {
                        class: "p-1.5 rounded-full hover:bg-white/10 text-gray-500 hover:text-beet-leaf transition-colors cursor-pointer",
                        title: "Tag this track with beets",
                        aria_label: "Tag this track with beets",
                        onclick: move |_| on_tag.call(()),
                        svg {
                            class: "w-3.5 h-3.5",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "2",
                            view_box: "0 0 24 24",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                d: "M16.862 3.487a2.25 2.25 0 113.182 3.182L7.5 19.213l-4.5 1.5 1.5-4.5L16.862 3.487z",
                            }
                        }
                    }
                    button {
                        class: "p-1.5 rounded-full hover:bg-white/10 text-gray-500 hover:text-beet-accent transition-colors cursor-pointer",
                        title: "Search Soulseek for this track",
                        aria_label: "Search Soulseek for this track",
                        onclick: move |_| on_search.call(()),
                        svg {
                            class: "w-3.5 h-3.5",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "2",
                            view_box: "0 0 24 24",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                d: "M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z",
                            }
                        }
                    }
                    button {
                        class: "p-1.5 rounded-full hover:bg-white/10 text-gray-500 hover:text-beet-leaf transition-colors cursor-pointer",
                        title: "Re-request this track",
                        aria_label: "Re-request this track",
                        onclick: move |_| on_request.call(()),
                        svg {
                            class: "w-3.5 h-3.5",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "2",
                            view_box: "0 0 24 24",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                d: "M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4",
                            }
                        }
                    }
                }
                button {
                    class: "p-1.5 rounded-full hover:bg-red-900/30 text-gray-500 hover:text-red-400 transition-colors cursor-pointer",
                    title: "Remove track and delete file",
                    aria_label: "Remove track and delete file",
                    onclick: move |_| on_remove.call(()),
                    svg {
                        class: "w-3.5 h-3.5",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            d: "M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673A2.25 2.25 0 0115.916 21H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0",
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn LibraryMissingRow(
    missing: LibraryMissingTrack,
    on_search: EventHandler,
    on_request: EventHandler,
) -> Element {
    let number = track_number_label(missing.disc_number, missing.track_number);

    rsx! {
        div { class: "flex items-center gap-3 p-2 rounded hover:bg-white/5 group",
            span { class: "w-[26px] shrink-0" }
            span { class: "text-[10px] font-mono text-gray-600 w-8 text-right shrink-0",
                "{number}"
            }
            div { class: "flex-1 min-w-0",
                p { class: "text-xs font-mono text-gray-500 break-words", "{missing.title}" }
                span {
                    class: "inline-block text-[9px] font-mono px-1.5 py-0.5 rounded border bg-red-600/20 text-red-300 border-red-500/40 mt-0.5",
                    "MISSING"
                }
            }
            div { class: "flex items-center gap-1 shrink-0",
                button {
                    class: "p-1.5 rounded-full hover:bg-white/10 text-gray-500 hover:text-beet-accent transition-colors cursor-pointer",
                    title: "Search Soulseek for this track",
                    aria_label: "Search Soulseek for this track",
                    onclick: move |_| on_search.call(()),
                    svg {
                        class: "w-3.5 h-3.5",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            d: "M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z",
                        }
                    }
                }
                button {
                    class: "p-1.5 rounded-full hover:bg-white/10 text-gray-500 hover:text-beet-leaf transition-colors cursor-pointer",
                    title: "Download this missing track",
                    aria_label: "Download this missing track",
                    onclick: move |_| on_request.call(()),
                    svg {
                        class: "w-3.5 h-3.5",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            d: "M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4",
                        }
                    }
                }
            }
        }
    }
}

fn release_quality_badge(release: &LibraryRelease) -> Element {
    let Some(format) = release.audio_format.as_deref() else {
        return rsx! {};
    };
    if release.quality_varies {
        return rsx! {
            QualityBadge { quality: "mixed".to_string(), label: Some("Mixed".to_string()) }
        };
    }
    rsx! {
        QualityBadge {
            quality: format.to_string(),
            bitrate: release.bitrate.map(|value| value as i32),
            bit_depth: release.bit_depth.map(|value| value as i32),
            sample_rate: release.sample_rate.map(|value| value as i32),
        }
    }
}

#[component]
pub fn LibraryTab(#[props(default)] focus: LibraryFocus) -> Element {
    let auth = use_auth();
    let mut overview =
        use_resource(move || async move { auth.call(api::get_library_overview()).await });

    let mut selected_folder = use_signal(|| None::<String>);
    let mut expanded = use_signal(HashSet::<String>::new);
    let mut details = use_signal(HashMap::<String, LibraryReleaseDetail>::new);
    let mut activity = use_signal(DownloadActivity::default);
    let mut last_tick = use_signal(|| 0u32);
    let mut ignored_overrides = use_signal(HashMap::<String, bool>::new);
    let main_artists = use_signal(HashMap::<String, String>::new);
    let mut pending = use_signal(|| None::<PendingRemoval>);
    let mut pending_request = use_signal(|| None::<PendingRequest>);
    let mut message = use_signal(String::new);
    let mut scanning = use_signal(|| false);
    let mut focus_applied = use_signal(|| false);

    let mut tag_preview = use_signal::<Option<LibraryTagPreview>>(|| None);
    let mut tag_folder_id = use_signal(String::new);
    let mut tag_candidate = use_signal(|| 0usize);
    let mut tag_selections = use_signal::<Vec<Option<usize>>>(Vec::new);
    let mut tag_loading = use_signal(|| false);

    use_future(move || async move {
        loop {
            if let Ok(next) = auth.call(api::get_download_activity()).await {
                if next != *activity.peek() {
                    activity.set(next);
                }
            }
            gloo_timers::future::TimeoutFuture::new(15_000).await;
        }
    });

    let download_tick = try_use_context::<DownloadTick>();
    use_effect(move || {
        let Some(tick) = download_tick else {
            return;
        };
        let value = (tick.0)();
        if value == 0 || value == *last_tick.peek() {
            return;
        }
        last_tick.set(value);
        let mut overview = overview;
        spawn(async move {
            if let Ok(next) = auth.call(api::get_download_activity()).await {
                activity.set(next);
            }
            overview.restart();
            gloo_timers::future::TimeoutFuture::new(2500).await;
            overview.restart();
            gloo_timers::future::TimeoutFuture::new(5000).await;
            overview.restart();
        });
    });

    use_effect(move || {
        if selected_folder.peek().is_some() {
            return;
        }
        if let Some(Ok(data)) = &*overview.read() {
            if let Some(folder) = data.folders.first() {
                selected_folder.set(Some(folder.id.clone()));
            }
        }
    });

    let folders = match &*overview.read() {
        Some(Ok(data)) => data.folders.clone(),
        _ => Vec::new(),
    };
    let releases: Vec<LibraryRelease> = match &*overview.read() {
        Some(Ok(data)) => data
            .releases
            .iter()
            .filter(|release| {
                selected_folder()
                    .as_ref()
                    .map(|folder| release.folder_id == *folder)
                    .unwrap_or(true)
            })
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    let selected_folder_path = selected_folder()
        .as_ref()
        .and_then(|id| folders.iter().find(|folder| folder.id == *id))
        .map(|folder| folder.path.clone())
        .unwrap_or_default();
    let selected_folder_name = selected_folder()
        .as_ref()
        .and_then(|id| folders.iter().find(|folder| folder.id == *id))
        .map(|folder| folder.name.clone())
        .unwrap_or_else(|| "this folder".to_string());
    let active_scan = selected_folder()
        .as_ref()
        .and_then(|id| folders.iter().find(|folder| folder.id == *id))
        .map(|folder| folder.scan_status == "scanning")
        .unwrap_or(false);

    let unknown_by_folder: BTreeMap<String, Vec<LibraryRelease>> = releases
        .iter()
        .filter(|release| is_unknown(release))
        .cloned()
        .fold(BTreeMap::new(), |mut map, release| {
            map.entry(release.folder_name.clone())
                .or_default()
                .push(release);
            map
        });
    let known_releases: Vec<LibraryRelease> = releases
        .iter()
        .filter(|release| !is_unknown(release))
        .cloned()
        .collect();
    let known_by_artist = {
        let overrides = main_artists.read().clone();
        group_by_artist(&known_releases, &overrides)
    };

    let scan_selected = move |_| {
        let Some(folder_id) = selected_folder() else {
            return;
        };
        scanning.set(true);
        message.set("Scanning library…".to_string());
        spawn(async move {
            match auth.call(api::scan_library_folder(folder_id)).await {
                Ok(result) => {
                    let text = if result.warnings.is_empty() {
                        format!(
                            "Indexed {} releases / {} tracks",
                            result.folder.release_count, result.folder.track_count
                        )
                    } else {
                        format!(
                            "Indexed {} releases / {} tracks · {}",
                            result.folder.release_count,
                            result.folder.track_count,
                            result.warnings.join(" · ")
                        )
                    };
                    message.set(text);
                }
                Err(e) => message.set(format!("Scan failed: {e}")),
            }
            scanning.set(false);
            overview.restart();
            details.write().clear();
        });
    };

    let scan_all = move |_| {
        scanning.set(true);
        message.set("Scanning all folders…".to_string());
        spawn(async move {
            match auth.call(api::scan_all_library_folders()).await {
                Ok(result) => {
                    message.set(format!(
                        "Indexed {} releases / {} tracks",
                        result.folder.release_count, result.folder.track_count
                    ));
                }
                Err(e) => message.set(format!("Scan failed: {e}")),
            }
            scanning.set(false);
            overview.restart();
            details.write().clear();
        });
    };

    let mut request_release = move |release: LibraryRelease| {
        message.set(String::new());
        let artist = release_artist(&release);
        if release.mbid.is_none() {
            open_search(&artist, &release.album, None, "album");
            return;
        }
        let album = Album {
            id: release.mbid.clone().unwrap_or_default(),
            title: release.album.clone(),
            artist,
            release_date: release.year.map(|year| year.to_string()),
            mbid: release.mbid.clone(),
            cover_url: None,
            country: None,
            media: None,
            track_count: Some(release.track_count),
        };
        let request = api::AutoDownloadRequest {
            query: DownloadQuery::new(vec![]).album(album),
            folder_id: release.folder_id.clone(),
            folder_path: release.folder_path.clone(),
        };
        spawn(async move {
            let mut overview = overview;
            if let Err(e) = auth
                .call(api::remove_library_release(release.id.clone()))
                .await
            {
                info!("Failed to remove release before re-request: {:?}", e);
            }
            match auth.call(api::auto_download(request)).await {
                Ok(api::AutoDownloadResult::Accepted { .. }) => {
                    message.set(format!("Requested a replacement for {}", release.album));
                }
                Ok(api::AutoDownloadResult::Error(e)) => {
                    message.set(format!("Could not request replacement: {e}"));
                }
                Err(e) => message.set(format!("Could not request replacement: {e}")),
            }
            details.write().clear();
            overview.restart();
        });
    };

    let mut request_track = move |release: LibraryRelease, track: LibraryTrackInfo| {
        message.set(String::new());
        let metadata = Track {
            id: track
                .mb_track_id
                .clone()
                .unwrap_or_else(|| format!("library:{}", track.id)),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album_id: track.mb_album_id.clone(),
            album_title: Some(track.album.clone()),
            release_date: None,
            duration: track
                .duration
                .map(|seconds| format!("{}:{:02}", seconds / 60, seconds % 60)),
            track_number: track.track_number,
            disc_number: track.disc_number,
            mbid: track.mb_track_id.clone(),
            release_mbid: track.mb_album_id.clone(),
        };
        let request = api::AutoDownloadRequest {
            query: DownloadQuery::from(metadata),
            folder_id: release.folder_id.clone(),
            folder_path: release.folder_path.clone(),
        };
        spawn(async move {
            let mut overview = overview;
            if let Err(e) = auth.call(api::remove_library_track(track.id.clone())).await {
                info!("Failed to remove track before re-request: {:?}", e);
            }
            match auth.call(api::auto_download(request)).await {
                Ok(api::AutoDownloadResult::Accepted { .. }) => {
                    message.set(format!("Requested a replacement for {}", track.title));
                }
                Ok(api::AutoDownloadResult::Error(e)) => {
                    message.set(format!("Could not request replacement: {e}"));
                }
                Err(e) => message.set(format!("Could not request replacement: {e}")),
            }
            details.write().clear();
            overview.restart();
        });
    };

    let mut request_missing_track = move |release: LibraryRelease, missing: LibraryMissingTrack| {
        message.set(String::new());
        let artist = if missing.artist.trim().is_empty() {
            release_artist(&release)
        } else {
            missing.artist.clone()
        };
        let metadata = Track {
            id: format!(
                "library-missing:{}:{}:{}",
                release.id,
                missing.disc_number.unwrap_or(1),
                missing.track_number.unwrap_or(0)
            ),
            title: missing.title.clone(),
            artist,
            album_id: release.mbid.clone(),
            album_title: Some(release.album.clone()),
            release_date: release.year.map(|year| year.to_string()),
            duration: None,
            track_number: missing.track_number,
            disc_number: missing.disc_number,
            mbid: None,
            release_mbid: release.mbid.clone(),
        };
        let request = api::AutoDownloadRequest {
            query: DownloadQuery::from(metadata),
            folder_id: release.folder_id.clone(),
            folder_path: release.folder_path.clone(),
        };
        spawn(async move {
            let mut overview = overview;
            match auth.call(api::auto_download(request)).await {
                Ok(api::AutoDownloadResult::Accepted { .. }) => {
                    message.set(format!("Requested {}", missing.title));
                }
                Ok(api::AutoDownloadResult::Error(e)) => {
                    message.set(format!("Could not request track: {e}"));
                }
                Err(e) => message.set(format!("Could not request track: {e}")),
            }
            details.write().clear();
            overview.restart();
        });
    };

    let mut confirm_request = move |_| {
        let Some(action) = pending_request() else {
            return;
        };
        pending_request.set(None);
        match action {
            PendingRequest::Release(release) => request_release(*release),
            PendingRequest::Track(release, track) => request_track(*release, *track),
            PendingRequest::Missing(release, missing) => request_missing_track(*release, *missing),
        }
    };

    let confirm_removal = move |_| {
        let Some(action) = pending() else {
            return;
        };
        pending.set(None);
        spawn(async move {
            let result = match action {
                PendingRemoval::Release(id) => auth.call(api::remove_library_release(id)).await,
                PendingRemoval::Track(id) => auth.call(api::remove_library_track(id)).await,
            };
            match result {
                Ok(result) => {
                    let mut text = format!("Removed {} file(s)", result.removed_files);
                    if result.removed_from_beets {
                        text.push_str(" · beets database updated");
                    }
                    if !result.failed_files.is_empty() {
                        text.push_str(&format!(
                            " · {} could not be deleted",
                            result.failed_files.len()
                        ));
                    }
                    message.set(text);
                }
                Err(e) => message.set(format!("Remove failed: {e}")),
            }
            overview.restart();
            details.write().clear();
        });
    };

    let mut ignore_release = move |(release_id, ignored): (String, bool)| {
        ignored_overrides.write().insert(release_id.clone(), ignored);
        let mut ignored_overrides = ignored_overrides;
        spawn(async move {
            if let Err(e) = auth
                .call(api::set_library_release_ignored(release_id.clone(), ignored))
                .await
            {
                ignored_overrides.write().remove(&release_id);
                message.set(format!("Could not update ignore flag: {e}"));
                return;
            }
            overview.restart();
        });
    };

    let mut open_tag_preview = move |request: LibraryTagPreviewRequest| {
        tag_loading.set(true);
        tag_preview.set(None);
        tag_folder_id.set(request.folder_id.clone());
        spawn(async move {
            match auth.call(api::preview_library_tagging(request)).await {
                Ok(preview) => {
                    let selections = preview
                        .files
                        .iter()
                        .map(|file| file.selected)
                        .collect::<Vec<_>>();
                    tag_candidate.set(preview.selected_candidate);
                    tag_selections.set(selections);
                    tag_preview.set(Some(preview));
                }
                Err(e) => message.set(format!("Could not load tagging candidates: {e}")),
            }
            tag_loading.set(false);
        });
    };

    let apply_tagging = move |_| {
        let Some(preview) = tag_preview.peek().clone() else {
            return;
        };
        let Some(candidate) = preview
            .candidates
            .get(tag_candidate.peek().to_owned())
            .cloned()
        else {
            return;
        };
        let selections = tag_selections.peek().clone();
        let tracks: Vec<LibraryTagApplyTrack> = preview
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                let track_index = selections.get(index).copied().flatten()?;
                let track = candidate.tracks.get(track_index)?;
                Some(LibraryTagApplyTrack {
                    path: file.path.clone(),
                    title: track.title.clone(),
                    artist: track.artist.clone(),
                    album: track.album.clone(),
                    album_artist: track.album_artist.clone(),
                    track_number: track.track_number,
                    disc_number: track.disc_number,
                    year: track.year,
                    mb_track_id: track.mb_track_id.clone(),
                    mb_album_id: track.mb_album_id.clone(),
                    item: None,
                })
            })
            .collect();
        if tracks.is_empty() {
            message.set("Select at least one track to tag".to_string());
            return;
        }
        let folder_id = tag_folder_id.peek().clone();
        tag_preview.set(None);
        spawn(async move {
            match auth
                .call(api::apply_library_tagging(LibraryTagApplyRequest {
                    folder_id,
                    tracks,
                }))
                .await
            {
                Ok(result) => {
                    if result.started {
                        message.set("Tagging started — see Active Transfers".to_string());
                    } else {
                        let mut text = format!("Tagged {} track(s)", result.updated);
                        if !result.beets_available {
                            text.push_str(" · beets library not found");
                        }
                        if !result.failures.is_empty() {
                            text.push_str(&format!(" · {} failed", result.failures.len()));
                            if let Some(first) = result.failures.first() {
                                text.push_str(&format!(" ({first})"));
                            }
                        }
                        message.set(text);
                    }
                }
                Err(e) => message.set(format!("Tagging failed: {e}")),
            }
            overview.restart();
            details.write().clear();
        });
    };

    let load_release = move |id: String| {
        let mut details = details;
        let mut main_artists = main_artists;
        let mut expanded = expanded;
        spawn(async move {
            match auth.call(api::get_library_release_detail(id.clone())).await {
                Ok(Some(detail)) => {
                    details.write().insert(id.clone(), detail);
                }
                Ok(None) => {}
                Err(e) => {
                    info!("Failed to load release detail: {:?}", e);
                    expanded.write().remove(&id);
                    return;
                }
            }
            match auth.call(api::get_library_release_extra(id.clone())).await {
                Ok(Some(extra)) => {
                    if let Some(artist) = extra.release.main_artist.clone() {
                        main_artists.write().insert(id.clone(), artist);
                    }
                    if let Some(detail) = details.write().get_mut(&id) {
                        detail.release = extra.release;
                        detail.missing_tracks = extra.missing_tracks;
                    }
                }
                Ok(None) => {}
                Err(e) => info!("Failed to load what the release is missing: {:?}", e),
            }
        });
    };

    let mut toggle_release = move |release: LibraryRelease| {
        let id = release.id.clone();
        let is_open = expanded.read().contains(&id);
        if is_open {
            expanded.write().remove(&id);
            return;
        }
        expanded.write().insert(id.clone());
        if details.peek().contains_key(&id) {
            return;
        }
        load_release(id);
    };

    use_effect(move || {
        if focus.is_empty() || focus_applied() {
            return;
        }
        let release = {
            let data = overview.read();
            match &*data {
                Some(Ok(data)) => data
                    .releases
                    .iter()
                    .find(|release| focus.matches(release))
                    .cloned(),
                _ => None,
            }
        };
        let Some(release) = release else {
            return;
        };

        focus_applied.set(true);
        selected_folder.set(Some(release.folder_id.clone()));
        let release_id = release.id.clone();
        expanded.write().insert(release_id.clone());
        if !details.peek().contains_key(&release_id) {
            load_release(release_id.clone());
        }
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(250).await;
            scroll_to_release(&release_id);
        });
    });

    let mut select_candidate = move |index: usize| {
        tag_candidate.set(index);
        let Some(preview) = tag_preview.peek().clone() else {
            return;
        };
        let Some(candidate) = preview.candidates.get(index) else {
            return;
        };
        let selections = preview
            .files
            .iter()
            .map(|file| suggest_track(&file.current_title, file.track_number, &candidate.tracks))
            .collect();
        tag_selections.set(selections);
    };

    let render_release = move |release: LibraryRelease| -> Element {
        let release_for_toggle = release.clone();
        let release_for_remove = release.clone();
        let release_for_request = release.clone();
        let release_for_search = release.clone();
        let release_for_ignore = release.clone();
        let release_for_tag = release.clone();
        let is_open = expanded.read().contains(&release.id);
        let detail = details.read().get(&release.id).cloned();
        let mut display_release = detail
            .as_ref()
            .map(|detail| detail.release.clone())
            .unwrap_or_else(|| release.clone());
        if let Some(ignored) = ignored_overrides.read().get(&release.id) {
            display_release.ignored = *ignored;
        }
        let album_artist = release_artist(&display_release);
        let artist_for_tag = album_artist.clone();
        let artist_for_search = album_artist.clone();
        let artist_for_request = album_artist.clone();
        let folder_path = display_release.folder_path.clone();
        let album_key = album_activity_key(&release.album_artist, &release.album);
        let activity_guard = activity.read();
        let download_state = activity_guard.albums.get(&album_key).cloned();
        let active_tracks = activity_guard
            .album_active_counts
            .get(&album_key)
            .copied()
            .unwrap_or(0);
        drop(activity_guard);
        let (badge, badge_class) =
            completeness_label(&display_release, active_tracks, download_state.is_some());
        let can_ignore = display_release.ignored
            || display_release.completeness != ReleaseCompleteness::Complete
            || display_release
                .expected_track_count
                .map(|expected| expected > display_release.track_count)
                .unwrap_or(false);
        let can_tag = is_unknown(&display_release)
            || display_release.matched_track_count < display_release.track_count;
        let is_complete = display_release.completeness == ReleaseCompleteness::Complete;
        let cover_urls: Vec<String> = [
            display_release
                .mbid
                .as_ref()
                .map(|mbid| format!("https://coverartarchive.org/release/{mbid}/front-250")),
            display_release
                .mb_release_group_id
                .as_ref()
                .map(|mbid| format!("https://coverartarchive.org/release-group/{mbid}/front-250")),
        ]
        .into_iter()
        .flatten()
        .collect();
        rsx! {
            div { key: "{release.id}",
                id: "library-release-{release.id}",
                class: "bg-white/5 border rounded-lg transition-colors",
                class: if is_open { "border-beet-accent/30" } else { "border-white/5 hover:border-white/10" },
                div {
                    class: "flex items-center gap-3 p-3 cursor-pointer",
                    onclick: move |_| toggle_release(release_for_toggle.clone()),
                    div { class: "w-12 h-12 shrink-0 bg-beet-panel border border-white/5 rounded overflow-hidden flex items-center justify-center",
                        CoverImage { urls: cover_urls }
                    }
                    div { class: "flex-grow min-w-0",
                        div { class: "flex items-center gap-2 min-w-0 flex-wrap",
                            p { class: "text-sm font-bold text-white break-words min-w-0",
                                "{release_title(&release)}"
                            }
                            if display_release.ignored {
                                button {
                                    class: "text-[10px] font-mono px-1.5 py-0.5 rounded border shrink-0 cursor-pointer {badge_class}",
                                    title: "Unignore release",
                                    onclick: {
                                        let release_id = release_for_ignore.id.clone();
                                        move |event: MouseEvent| {
                                            event.stop_propagation();
                                            ignore_release((release_id.clone(), false));
                                        }
                                    },
                                    "{badge}"
                                }
                            } else {
                                span {
                                    class: "text-[10px] font-mono px-1.5 py-0.5 rounded border shrink-0 {badge_class}",
                                    "{badge}"
                                }
                            }
                            {release_quality_badge(&release)}
                            {download_state_badge(download_state.as_ref())}
                        }
                        p { class: "text-xs text-gray-400 font-mono break-words mt-1",
                            "{release_subtitle(&release)}"
                        }
                        div { class: "flex flex-wrap gap-x-3 gap-y-0.5 text-[10px] font-mono text-gray-600 mt-0.5",
                            span { "{format_size(release.total_size)}" }
                        }
                    }
                    svg {
                        class: "w-4 h-4 shrink-0 transition-transform",
                        class: if is_open { "rotate-90 text-beet-accent" } else { "text-gray-500" },
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            d: "M8.25 4.5l7.5 7.5-7.5 7.5",
                        }
                    }
                    div { class: "flex items-center gap-1 shrink-0",
                        if can_tag {
                            button {
                                class: "px-2 py-1 text-[10px] font-mono rounded border border-beet-leaf/40 text-beet-leaf hover:bg-beet-leaf/10 transition-colors cursor-pointer",
                                title: "Tag this release with beets",
                                onclick: move |event: MouseEvent| {
                                    event.stop_propagation();
                                    open_tag_preview(LibraryTagPreviewRequest {
                                        folder_id: release_for_tag.folder_id.clone(),
                                        release_id: Some(release_for_tag.id.clone()),
                                        path: release_for_tag.path.clone(),
                                        artist: Some(artist_for_tag.clone()),
                                        album: Some(release_for_tag.album.clone()),
                                        is_track: false,
                                    });
                                },
                                "TAG"
                            }
                        }
                        if can_ignore && !display_release.ignored {
                            button {
                                class: "px-2 py-1 text-[10px] font-mono rounded border transition-colors cursor-pointer border-gray-600 text-gray-500 hover:text-gray-300 hover:bg-white/5",
                                title: "Ignore incomplete release",
                                onclick: {
                                    let release_id = release_for_ignore.id.clone();
                                    move |event: MouseEvent| {
                                        event.stop_propagation();
                                        ignore_release((release_id.clone(), true));
                                    }
                                },
                                "IGNORE"
                            }
                        }
                        if !is_complete {
                            button {
                                class: "p-2 rounded-full hover:bg-white/10 text-gray-500 hover:text-beet-accent transition-colors cursor-pointer",
                                title: "Find a replacement on Soulseek",
                                onclick: move |event: MouseEvent| {
                                    event.stop_propagation();
                                    open_search(
                                        &artist_for_search,
                                        &release_for_search.album,
                                        release_for_search.mbid.as_deref(),
                                        "album",
                                    );
                                },
                                svg {
                                    class: "w-4 h-4",
                                    fill: "none",
                                    stroke: "currentColor",
                                    stroke_width: "2",
                                    view_box: "0 0 24 24",
                                    path {
                                        stroke_linecap: "round",
                                        stroke_linejoin: "round",
                                        d: "M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z",
                                    }
                                }
                            }
                        }
                        if !is_complete {
                            button {
                                class: "p-2 rounded-full hover:bg-white/10 text-gray-500 hover:text-beet-leaf transition-colors cursor-pointer",
                                title: "Re-request this release",
                                onclick: move |event: MouseEvent| {
                                    event.stop_propagation();
                                    if release_for_request.mbid.is_none() {
                                        open_search(
                                            &artist_for_request,
                                            &release_for_request.album,
                                            None,
                                            "album",
                                        );
                                    } else {
                                        pending_request.set(Some(PendingRequest::Release(Box::new(
                                            release_for_request.clone(),
                                        ))));
                                    }
                                },
                                svg {
                                    class: "w-4 h-4",
                                    fill: "none",
                                    stroke: "currentColor",
                                    stroke_width: "2",
                                    view_box: "0 0 24 24",
                                    path {
                                        stroke_linecap: "round",
                                        stroke_linejoin: "round",
                                        d: "M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4",
                                    }
                                }
                            }
                        }
                        button {
                            class: "p-2 rounded-full hover:bg-red-900/30 text-gray-500 hover:text-red-400 transition-colors cursor-pointer",
                            title: "Remove release and delete files",
                            onclick: move |event: MouseEvent| {
                                event.stop_propagation();
                                pending.set(Some(PendingRemoval::Release(release_for_remove.id.clone())));
                            },
                            svg {
                                class: "w-4 h-4",
                                fill: "none",
                                stroke: "currentColor",
                                stroke_width: "2",
                                view_box: "0 0 24 24",
                                path {
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                    d: "M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673A2.25 2.25 0 0115.916 21H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0",
                                }
                            }
                        }
                    }
                }
                if is_open {
                    div { class: "border-t border-white/10 bg-beet-dark/50 p-3 space-y-1",
                        match detail {
                            None => rsx! {
                                p { class: "text-xs font-mono text-gray-500 animate-pulse py-2",
                                    "Loading tracks…"
                                }
                            },
                            Some(detail) => rsx! {
                                for row in ordered_library_rows(&detail) {
                                    match row {
                                        LibraryRow::Track(track) => {
                                            let track_for_row = track.clone();
                                            let track_for_tag = track.clone();
                                            let track_for_search = track.clone();
                                            let track_for_request = track.clone();
                                            let track_for_remove = track.clone();
                                            let release_for_tag = release.clone();
                                            let release_for_request = release.clone();
                                            let track_download_state = activity
                                                .read()
                                                .tracks
                                                .get(&track_activity_key(&track.artist, &track.title))
                                                .cloned();
                                            rsx! {
                                                LibraryTrackRow {
                                                    track: track_for_row,
                                                    folder_path: folder_path.clone(),
                                                    download_state: track_download_state,
                                                    on_tag: move |_| open_tag_preview(LibraryTagPreviewRequest {
                                                        folder_id: release_for_tag.folder_id.clone(),
                                                        release_id: Some(release_for_tag.id.clone()),
                                                        path: track_for_tag.path.clone(),
                                                        artist: Some(track_for_tag.artist.clone()),
                                                        album: Some(track_for_tag.title.clone()),
                                                        is_track: true,
                                                    }),
                                                    on_search: move |_| open_search(
                                                        &track_for_search.artist,
                                                        &track_for_search.title,
                                                        None,
                                                        "track",
                                                    ),
                                                    on_request: move |_| pending_request.set(Some(PendingRequest::Track(
                                                        Box::new(release_for_request.clone()),
                                                        Box::new(track_for_request.clone()),
                                                    ))),
                                                    on_remove: move |_| pending.set(Some(PendingRemoval::Track(track_for_remove.id.clone()))),
                                                }
                                            }
                                        }
                                        LibraryRow::Missing(missing) => {
                                            let missing_for_row = missing.clone();
                                            let missing_for_search = missing.clone();
                                            let missing_for_request = missing.clone();
                                            let release_for_search = release.clone();
                                            let release_for_request = release.clone();
                                            rsx! {
                                                LibraryMissingRow {
                                                    missing: missing_for_row,
                                                    on_search: move |_| open_search(
                                                        &missing_for_search.artist,
                                                        &missing_for_search.title,
                                                        release_for_search.mbid.as_deref(),
                                                        "track",
                                                    ),
                                                    on_request: move |_| pending_request.set(Some(PendingRequest::Missing(
                                                        Box::new(release_for_request.clone()),
                                                        Box::new(missing_for_request.clone()),
                                                    ))),
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                        }
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "space-y-4",
            div { class: "flex flex-wrap items-center gap-2 justify-between",
                div { class: "flex items-center gap-2 min-w-0",
                    label { class: "text-[10px] font-mono uppercase tracking-widest text-gray-500 shrink-0",
                        "Library"
                    }
                    select {
                        class: "bg-beet-dark border border-white/10 rounded px-2 py-1 text-xs font-mono text-gray-300 focus:border-beet-accent focus:outline-none max-w-[18rem]",
                        value: "{selected_folder().unwrap_or_default()}",
                        onchange: move |event| {
                            let value = event.value();
                            selected_folder.set((!value.is_empty()).then_some(value));
                            details.write().clear();
                        },
                        for folder in folders.iter() {
                            option {
                                value: "{folder.id}",
                                selected: selected_folder().as_deref() == Some(folder.id.as_str()),
                                "{folder.name} · {folder.release_count} releases"
                            }
                        }
                    }
                    span { class: "text-[10px] font-mono uppercase tracking-widest text-gray-500 shrink-0",
                        "({releases.len()} releases)"
                    }
                }
                div { class: "flex items-center gap-2",
                    span { class: "text-[10px] font-mono text-gray-600 truncate hidden sm:inline max-w-[16rem]",
                        "{selected_folder_path}"
                    }
                    button {
                        class: "px-3 py-1.5 text-xs font-mono rounded border border-white/10 text-gray-300 hover:text-white hover:bg-white/5 transition-colors cursor-pointer disabled:opacity-40",
                        disabled: scanning() || selected_folder().is_none(),
                        onclick: scan_selected,
                        if scanning() || active_scan { "Scanning…" } else { "Rescan" }
                    }
                    button {
                        class: "px-3 py-1.5 text-xs font-mono rounded border border-white/10 text-gray-300 hover:text-white hover:bg-white/5 transition-colors cursor-pointer disabled:opacity-40",
                        disabled: scanning(),
                        onclick: scan_all,
                        "Scan all"
                    }
                }
            }

            if !message().is_empty() {
                p { class: "text-xs font-mono text-gray-400 bg-beet-panel/40 border border-white/5 rounded px-3 py-2",
                    "{message}"
                }
            }

            match &*overview.read() {
                None => rsx! {
                    div { class: "flex justify-center py-10",
                        div { class: "animate-spin rounded-full h-10 w-10 border-t-2 border-b-2 border-beet-accent" }
                    }
                },
                Some(Err(e)) => rsx! {
                    p { class: "text-red-400 font-mono text-sm", "Failed to load library: {e}" }
                },
                Some(Ok(_)) if releases.is_empty() => rsx! {
                    div { class: "text-center py-12 space-y-2",
                        p { class: "text-gray-400 font-mono text-sm",
                            "No releases indexed in {selected_folder_name} yet."
                        }
                        p { class: "text-gray-600 font-mono text-xs",
                            "Press Rescan to index this folder."
                        }
                    }
                },
                Some(Ok(_)) => rsx! {
                    div { class: "space-y-4",
                        if !unknown_by_folder.is_empty() {
                            section { class: "space-y-2",
                                div { class: "flex items-center justify-between text-[10px] font-mono uppercase tracking-widest text-gray-500 px-1",
                                    span { "Unknown" }
                                    span { "Folder · unmatched files" }
                                }
                                for (folder, group) in unknown_by_folder.iter() {
                                    div { class: "space-y-2 pt-1",
                                        div { class: "flex items-center gap-2 px-1",
                                            span { class: "text-[10px] font-mono uppercase tracking-widest text-gray-600", "{folder}" }
                                            span { class: "text-[10px] font-mono text-gray-700", "· {group.len()} release(s)" }
                                        }
                                        for release in group.iter() {
                                            {render_release(release.clone())}
                                        }
                                    }
                                }
                            }
                        }

                        if !known_by_artist.is_empty() {
                            section { class: "space-y-3",
                                for (artist, group) in known_by_artist.iter() {
                                    div { class: "space-y-2 pt-1",
                                        div { class: "flex items-center gap-2 px-1",
                                            span { class: "text-[10px] font-mono uppercase tracking-widest text-gray-500", "{artist}" }
                                            span { class: "text-[10px] font-mono text-gray-700", "· {group.len()} release(s)" }
                                        }
                                        for release in group.iter() {
                                            {render_release(release.clone())}
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }

        if let Some(preview) = tag_preview() {
            {
                let selected = tag_candidate().min(preview.candidates.len().saturating_sub(1));
                let candidate = preview.candidates.get(selected).cloned();
                rsx! {
                    Modal {
                        on_close: move |_| tag_preview.set(None),
                        header: rsx! {
                            div {
                                h2 { class: "text-lg font-bold text-white font-display", "Tag with beets" }
                                p { class: "text-[10px] font-mono text-gray-500 uppercase tracking-widest",
                                    "Pick the matching release, then confirm each file"
                                }
                            }
                        },
                        if preview.candidates.is_empty() {
                            p { class: "text-sm font-mono text-gray-400 py-6 text-center",
                                "No metadata candidates found for this folder."
                            }
                        } else {
                            div { class: "space-y-3",
                                div {
                                    label { class: "block text-[10px] font-mono uppercase tracking-widest text-gray-500 mb-1",
                                        "Release"
                                    }
                                    select {
                                        class: "w-full bg-beet-dark border border-white/10 rounded px-2 py-1.5 text-xs font-mono text-gray-300 focus:border-beet-accent focus:outline-none",
                                        value: "{selected}",
                                        onchange: move |event| {
                                            if let Ok(index) = event.value().parse::<usize>() {
                                                select_candidate(index);
                                            }
                                        },
                                        for (index, option) in preview.candidates.iter().enumerate() {
                                            option {
                                                value: "{index}",
                                                selected: index == selected,
                                                "{option.artist} — {option.title} ({option.track_count})"
                                            }
                                        }
                                    }
                                }

                                if let Some(candidate) = candidate {
                                    div { class: "space-y-1 rounded border border-white/5 bg-beet-dark/40 p-2",
                                        for (index, file) in preview.files.iter().enumerate() {
                                            {
                                                let selections = tag_selections.read().clone();
                                                let current = selections.get(index).copied().flatten();
                                                rsx! {
                                                    div { class: "p-1.5 rounded hover:bg-white/5 space-y-1.5",
                                                        p { class: "text-xs font-mono text-gray-300 truncate", "{file.name}" }
                                                        div { class: "flex items-center gap-2 min-w-0",
                                                            span { class: "text-gray-600 font-mono shrink-0", ">>>" }
                                                            select {
                                                                class: "flex-1 min-w-0 bg-beet-panel border border-white/10 rounded px-2 py-1 text-xs font-mono text-gray-300 focus:border-beet-accent focus:outline-none",
                                                                value: "{current.map(|v| v.to_string()).unwrap_or_default()}",
                                                                onchange: move |event| {
                                                                    let value = event.value();
                                                                    let mut selections = tag_selections.write();
                                                                    if selections.len() <= index {
                                                                        selections.resize(index + 1, None);
                                                                    }
                                                                    selections[index] = value.parse::<usize>().ok();
                                                                },
                                                                option { value: "", "— skip —" }
                                                                for (track_index, track) in candidate.tracks.iter().enumerate() {
                                                                    option {
                                                                        value: "{track_index}",
                                                                        selected: current == Some(track_index),
                                                                        {
                                                                            let prefix = track.track_number.map(|n| format!("{n:02} ")).unwrap_or_default();
                                                                            format!("{prefix}{}", track.title)
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                div { class: "flex justify-end gap-2 pt-1",
                                    button {
                                        class: "px-3 py-1.5 text-sm font-mono text-gray-400 hover:text-white transition-colors cursor-pointer",
                                        onclick: move |_| tag_preview.set(None),
                                        "Cancel"
                                    }
                                    button {
                                        class: "px-3 py-1.5 text-sm font-mono rounded bg-beet-leaf/20 text-beet-leaf hover:bg-beet-leaf/30 transition-colors cursor-pointer",
                                        onclick: apply_tagging,
                                        "Apply tags"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(action) = pending_request() {
            {
                let danger = !matches!(&action, PendingRequest::Missing(..));
                let (message, confirm_label) = match &action {
                    PendingRequest::Release(release) => (
                        format!(
                            "Re-request {}? The existing files will be deleted first, then a fresh copy will be downloaded.",
                            release.album
                        ),
                        "Re-request",
                    ),
                    PendingRequest::Track(_, track) => (
                        format!(
                            "Re-request {}? The existing file will be deleted first, then a fresh copy will be downloaded.",
                            track.title
                        ),
                        "Re-request",
                    ),
                    PendingRequest::Missing(_, track) => (
                        format!("Download the missing track {}?", track.title),
                        "Download",
                    ),
                };
                rsx! {
                    ConfirmModal {
                        message,
                        confirm_label: confirm_label.to_string(),
                        danger,
                        on_confirm: move |_| confirm_request(()),
                        on_cancel: move |_| pending_request.set(None),
                    }
                }
            }
        }

        if let Some(action) = pending() {
            {
                let message = match action {
                    PendingRemoval::Release(_) => "Remove this release and delete all of its files? This cannot be undone.".to_string(),
                    PendingRemoval::Track(_) => "Remove this track and delete its file? This cannot be undone.".to_string(),
                };
                rsx! {
                    ConfirmModal {
                        message,
                        confirm_label: "Remove".to_string(),
                        danger: true,
                        on_confirm: confirm_removal,
                        on_cancel: move |_| pending.set(None),
                    }
                }
            }
        }

        if tag_loading() {
            div { class: "fixed bottom-4 right-4 z-50 px-3 py-2 rounded bg-beet-panel border border-white/10 text-xs font-mono text-gray-300",
                "Loading metadata candidates…"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(album: &str, disambig: Option<&str>, media: Option<&str>) -> LibraryRelease {
        LibraryRelease {
            album: album.to_string(),
            album_artist: "Ed Sheeran".to_string(),
            album_disambig: disambig.map(str::to_string),
            media: media.map(str::to_string),
            country: Some("GB".to_string()),
            year: Some(2017),
            ..LibraryRelease::default()
        }
    }

    #[test]
    fn the_title_names_the_pressing_musicbrainz_distinguishes() {
        assert_eq!(
            release_title(&release("÷", Some("deluxe edition"), None)),
            "÷ (deluxe edition)"
        );
        assert_eq!(release_title(&release("÷", None, None)), "÷");
        assert_eq!(release_title(&release("÷", Some("  "), None)), "÷");
    }

    #[test]
    fn the_subtitle_carries_the_edition_facts_instead_of_the_folder() {
        assert_eq!(
            release_subtitle(&release("÷", None, Some("CD"))),
            "2017 · CD · GB"
        );
        assert_eq!(release_subtitle(&release("÷", None, None)), "2017 · GB");
    }

    #[test]
    fn the_artist_is_the_first_one_musicbrainz_credits() {        let mut featured = release("Singles", None, None);
        featured.album_artist = "5 Seconds of Summer feat. Julia Michaels".into();
        featured.main_artist = Some("5 Seconds of Summer".into());
        assert_eq!(release_artist(&featured), "5 Seconds of Summer");

        let mut unfiled = release("÷", None, None);
        unfiled.album_artist = "Ed Sheeran".into();
        assert_eq!(release_artist(&unfiled), "Ed Sheeran");
    }

    #[test]
    fn releases_are_grouped_under_the_artist_musicbrainz_names_first() {
        let mut featured = release("Singles", None, None);
        featured.id = "featured".into();
        featured.album_artist = "5 Seconds of Summer feat. Julia Michaels".into();
        featured.main_artist = Some("5 Seconds of Summer".into());
        let mut other = release("÷", None, None);
        other.id = "other".into();

        let groups = group_by_artist(&[featured, other], &HashMap::new());
        assert_eq!(
            groups.keys().collect::<Vec<_>>(),
            ["5 Seconds of Summer", "Ed Sheeran"]
        );

        let mut learned = release("Singles", None, None);
        learned.id = "learned".into();
        learned.album_artist = "5 Seconds of Summer feat. Julia Michaels".into();
        let overrides = HashMap::from([("learned".to_string(), "5 Seconds of Summer".to_string())]);
        let groups = group_by_artist(&[learned], &overrides);
        assert_eq!(
            groups.keys().collect::<Vec<_>>(),
            ["5 Seconds of Summer"]
        );
    }

    #[test]
    fn track_paths_drop_the_library_prefix() {
        assert_eq!(
            relative_track_path("/music/Ed Sheeran/÷/01.flac", "/music"),
            "Ed Sheeran/÷/01.flac"
        );
        assert_eq!(
            relative_track_path("/other/01.flac", "/music"),
            "other/01.flac"
        );
    }

    #[test]
    fn missing_tracks_keep_their_place_between_present_tracks() {
        let detail = LibraryReleaseDetail {
            tracks: vec![
                LibraryTrackInfo {
                    id: "3".into(),
                    track_number: Some(3),
                    title: "Third".into(),
                    ..LibraryTrackInfo::default()
                },
                LibraryTrackInfo {
                    id: "1".into(),
                    track_number: Some(1),
                    title: "First".into(),
                    ..LibraryTrackInfo::default()
                },
            ],
            missing_tracks: vec![LibraryMissingTrack {
                track_number: Some(2),
                title: "Second".into(),
                ..LibraryMissingTrack::default()
            }],
            ..LibraryReleaseDetail::default()
        };

        let rows = ordered_library_rows(&detail);
        let titles: Vec<&str> = rows.iter().map(LibraryRow::title).collect();
        assert_eq!(titles, ["First", "Second", "Third"]);
        assert!(matches!(&rows[1], LibraryRow::Missing(_)));
    }

    #[test]
    fn a_library_focus_matches_an_mbid_or_the_artist_and_album() {
        let target = LibraryRelease {
            id: "release-id".into(),
            mbid: Some("mbid".into()),
            album: "÷".into(),
            album_artist: "Ed Sheeran".into(),
            ..LibraryRelease::default()
        };
        assert!(LibraryFocus {
            release: Some("mbid".into()),
            ..LibraryFocus::default()
        }
        .matches(&target));
        assert!(LibraryFocus {
            artist: Some("ed sheeran".into()),
            album: Some("÷".into()),
            ..LibraryFocus::default()
        }
        .matches(&target));
        assert!(!LibraryFocus {
            artist: Some("Someone Else".into()),
            album: Some("÷".into()),
            ..LibraryFocus::default()
        }
        .matches(&target));
    }
}
