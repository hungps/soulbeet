use dioxus::logger::tracing::info;
use dioxus::prelude::*;
use shared::download::{DownloadRequestContext, DownloadableGroup, DownloadableItem};
use std::collections::HashSet;

use crate::{use_auth, Checkbox, QualityBadge, FIELD_CLASS};

#[derive(Props, PartialEq, Clone)]
pub struct Props {
    pub results: Vec<DownloadableGroup>,
    pub is_searching: bool,
    pub is_downloading: Signal<bool>,
    pub request: DownloadRequestContext,
    #[props(default)]
    pub selected_folder_id: Option<String>,
    #[props(into)]
    pub on_download: EventHandler<(Vec<DownloadableItem>, String, DownloadRequestContext)>,
    #[props(into)]
    pub on_back: EventHandler<()>,
}

#[derive(Props, Clone, PartialEq)]
struct AlbumResultItemProps {
    album: DownloadableGroup,
    selected_tracks: Signal<HashSet<String>>,
    on_album_select_all: EventHandler<DownloadableGroup>,
    on_track_toggle: EventHandler<String>,
    on_download_group: EventHandler<DownloadableGroup>,
    request: DownloadRequestContext,
    can_download: bool,
    is_best: bool,
    starts_expanded: bool,
}

#[derive(Props, Clone, PartialEq)]
struct TrackItemProps {
    track: DownloadableItem,
    is_selected: bool,
    on_toggle: EventHandler<String>,
}

fn get_track_id(track: &DownloadableItem) -> String {
    format!("{}{}", track.id, track.source)
}

fn group_key(group: &DownloadableGroup) -> String {
    format!("{}::{}", group.source, group.group_id)
}

/// The file name a peer served, without its path.
fn file_name(item: &DownloadableItem) -> String {
    let normalized = item.id.replace('\\', "/");
    normalized
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(&normalized)
        .to_string()
}

/// How much of the release the folder holds, e.g. `18 of 20 tracks`.
fn track_fit(group: &DownloadableGroup, request: &DownloadRequestContext) -> String {
    match request.track_count {
        Some(total) => format!("{} of {} tracks", group.item_count, total),
        None => format!("{} tracks", group.item_count),
    }
}

/// The selected edition's country, medium and date, as the album card shows them.
fn selected_release_facts(request: &DownloadRequestContext) -> String {
    [
        request.release.country.clone(),
        request.release.media.clone(),
        request.release.release_date.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<String>>()
    .join(" · ")
}

/// The line under the folder path: who serves it, how it matches, its score.
fn release_facts(group: &DownloadableGroup, request: &DownloadRequestContext) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(artist) = group.artist.as_ref().filter(|artist| !artist.is_empty()) {
        parts.push(artist.clone());
    }
    if !group.quality.is_empty() {
        parts.push(group.quality.to_uppercase());
    }
    parts.push(track_fit(group, request));
    if group.file_count > group.item_count {
        parts.push(format!("{} files in folder", group.file_count));
    }
    parts.push(format!("Score {:.2}", group.score));
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(item_count: usize, file_count: usize) -> DownloadableGroup {
        DownloadableGroup {
            source: "peer".to_string(),
            group_id: "/music/Ed Sheeran - ÷ (Deluxe)".to_string(),
            title: "Ed Sheeran - ÷ (Deluxe)".to_string(),
            artist: Some("Ed Sheeran".to_string()),
            item_count,
            total_size: 0,
            items: Vec::new(),
            quality: "flac".to_string(),
            file_count,
            release_fit: 1.0,
            score: 0.93,
        }
    }

    fn request(track_count: Option<u32>) -> DownloadRequestContext {
        DownloadRequestContext {
            key: "album:1".to_string(),
            label: "÷ (Deluxe)".to_string(),
            artist: Some("Ed Sheeran".to_string()),
            track_count,
            release: Default::default(),
        }
    }

    #[test]
    fn selected_release_facts_join_the_known_edition_parts() {
        let mut req = request(Some(1));
        assert_eq!(selected_release_facts(&req), "");
        req.release.country = Some("SE".into());
        req.release.media = Some("CD".into());
        req.release.release_date = Some("2014-12-01".into());
        assert_eq!(selected_release_facts(&req), "SE · CD · 2014-12-01");
    }

    #[test]
    fn facts_compare_the_folder_with_the_release() {
        assert_eq!(
            release_facts(&group(18, 18), &request(Some(18))),
            "Ed Sheeran · FLAC · 18 of 18 tracks · Score 0.93"
        );
    }

    #[test]
    fn facts_name_the_extra_files_of_another_edition() {
        assert!(release_facts(&group(12, 20), &request(Some(12)))
            .contains("12 of 12 tracks · 20 files in folder"));
    }

    #[test]
    fn facts_drop_the_release_count_when_it_is_unknown() {
        assert!(release_facts(&group(12, 12), &request(None)).contains("12 tracks"));
    }

    #[test]
    fn a_tracks_file_name_is_the_last_segment_of_its_id() {
        let item = |id: &str| DownloadableItem {
            id: id.to_string(),
            source: "peer".to_string(),
            title: "Eraser".to_string(),
            artist: "Ed Sheeran".to_string(),
            album: "÷".to_string(),
            size: None,
            duration: None,
            quality: "flac".to_string(),
            quality_score: 1.0,
            backend_data: None,
        };
        assert_eq!(
            file_name(&item("Music\\÷\\01 - Eraser.flac")),
            "01 - Eraser.flac"
        );
        assert_eq!(file_name(&item("Eraser.flac")), "Eraser.flac");
    }
}

#[component]
fn TrackItem(props: TrackItemProps) -> Element {
    let unique_id = get_track_id(&props.track);

    rsx! {
        li {
            key: "{unique_id}",
            class: "flex items-center gap-2 p-1 rounded-md hover:bg-white/10 cursor-pointer",
            onclick: move |_| props.on_toggle.call(unique_id.clone()),

            Checkbox { is_selected: props.is_selected }

            label { class: "cursor-pointer text-gray-300 font-mono text-sm flex-1 min-w-0",
                span { class: "block break-words", "{props.track.title}" }
                span { class: "block break-all text-[11px] text-gray-500", title: "{props.track.id}",
                    "{file_name(&props.track)}"
                }
            }

            QualityBadge {
                quality: props.track.quality.clone(),
                backend_data: props.track.backend_data.clone(),
            }
        }
    }
}

#[component]
fn AlbumResultItem(props: AlbumResultItemProps) -> Element {
    let album = props.album.clone();
    let group_for_download = album.clone();
    let folder_path = album.group_id.clone();
    let key = group_key(&album);
    let facts = release_facts(&album, &props.request);
    let mut is_expanded = use_signal(|| props.starts_expanded);

    rsx! {
        div {
            key: "{key}",
            class: "bg-white/5 border border-white/5 p-4 rounded-md",

            // Clickable header for expand/collapse
            div {
                class: "flex justify-between items-start gap-2 mb-2 cursor-pointer",
                onclick: move |_| is_expanded.toggle(),

                div { class: "flex-grow min-w-0",
                    div { class: "flex items-center gap-2 min-w-0",
                        h4 { class: "text-md font-bold text-beet-leaf break-words min-w-0", "{album.title}" }
                        if props.is_best {
                            span {
                                class: "text-[10px] font-mono px-1.5 py-0.5 rounded border bg-beet-accent/20 text-beet-accent border-beet-accent/40 uppercase shrink-0",
                                "Best match"
                            }
                        }
                    }
                    p { class: "text-[11px] text-gray-500 font-mono break-all", "{folder_path}" }
                    p { class: "text-sm text-gray-400 font-mono break-words", "{facts}" }
                }

                // Download the whole release
                button {
                    class: "p-1.5 rounded-full hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed shrink-0",
                    aria_label: "Download release",
                    title: "Download this release",
                    disabled: !props.can_download,
                    onclick: move |evt: MouseEvent| {
                        evt.stop_propagation();
                        props.on_download_group.call(group_for_download.clone());
                    },
                    svg {
                        class: "w-4 h-4 text-gray-400",
                        fill: "none",
                        stroke: "currentColor",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            stroke_width: "2",
                            d: "M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4",
                        }
                    }
                }

                // Expand/collapse chevron
                svg {
                    class: "w-4 h-4 transition-transform duration-200 shrink-0",
                    class: if *is_expanded.read() { "rotate-90 text-gray-400" } else { "text-gray-600" },
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
            }

            // Track list and Select All only visible when expanded
            if *is_expanded.read() {
                button {
                    class: "font-mono uppercase text-[10px] whitespace-nowrap tracking-widest px-3 py-1 mb-2 border border-beet-leaf/30 text-beet-leaf hover:bg-beet-leaf hover:text-beet-dark transition-colors cursor-pointer rounded",
                    onclick: move |_| props.on_album_select_all.call(album.clone()),
                    "Select All"
                }
                ul { class: "space-y-1",
                    for track in props.album.items {
                        TrackItem {
                            is_selected: props.selected_tracks.read().contains(&get_track_id(&track)),
                            track,
                            on_toggle: props.on_track_toggle,
                        }
                    }
                }
            }
        }
    }
}

/// Main component responsible for displaying all download options.
#[component]
pub fn DownloadResults(props: Props) -> Element {
    let mut selected_tracks = use_signal(HashSet::<String>::new);
    let mut sorted_results = props.results.clone();
    sorted_results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.release_fit
                    .partial_cmp(&a.release_fit)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| group_key(a).cmp(&group_key(b)))
    });
    let results = sorted_results;
    let selected_release = props.request.clone();
    let selected_release_meta = selected_release_facts(&selected_release);
    let item_request = props.request.clone();
    let preferred_folder_id = props.selected_folder_id.clone();
    let mut folders = use_signal(std::vec::Vec::new);
    let mut selected_folder = use_signal(String::new);
    let mut is_downloading = props.is_downloading;
    let auth = use_auth();

    use_future(move || async move {
        if let Ok(user_folders) = auth.call(api::get_user_folders()).await {
            info!("Fetched {} user folders", user_folders.len());
            folders.set(user_folders);
        }
    });

    use_effect(move || {
        let folder_list = folders.read();
        if folder_list.is_empty() {
            return;
        }
        let preferred = preferred_folder_id
            .as_ref()
            .and_then(|id| folder_list.iter().find(|folder| &folder.id == id))
            .or_else(|| (folder_list.len() == 1).then(|| &folder_list[0]));
        let Some(folder) = preferred else {
            return;
        };
        let already_selected = !selected_folder.peek().is_empty();
        if !already_selected {
            selected_folder.set(folder.path.clone());
        }
    });

    let handle_album_select_all = move |group: DownloadableGroup| {
        let mut selected = selected_tracks.write();
        let all_selected = group
            .items
            .iter()
            .all(|t| selected.contains(&get_track_id(t)));

        if all_selected {
            for item in &group.items {
                selected.remove(&get_track_id(item));
            }
        } else {
            for item in &group.items {
                selected.insert(get_track_id(item));
            }
        }
    };

    let handle_track_toggle = move |filename: String| {
        info!("Toggle track selection: {}", filename);
        let mut selected = selected_tracks.write();
        if selected.contains(&filename) {
            selected.remove(&filename);
        } else {
            selected.insert(filename);
        }
    };

    let handle_download_group = EventHandler::new({
        let request = props.request.clone();
        let on_download = props.on_download;
        move |group: DownloadableGroup| {
            if *is_downloading.read() || selected_folder.read().is_empty() {
                return;
            }
            let items = group.items.clone();
            if items.is_empty() {
                return;
            }
            is_downloading.set(true);
            on_download.call((items, selected_folder(), request.clone()));
        }
    });

    let handle_download = move |_| {
        // Prevent double-clicks by checking if already downloading
        if *is_downloading.read() {
            info!("Download already in progress, ignoring click");
            return;
        }

        let selected_ids = selected_tracks.read();

        let items_to_download: Vec<DownloadableItem> = props
            .results
            .iter()
            .flat_map(|group| group.items.iter())
            .filter(|item| selected_ids.contains(&get_track_id(item)))
            .cloned()
            .collect();

        if items_to_download.is_empty() {
            return;
        }

        // Set downloading state immediately to prevent double-clicks
        is_downloading.set(true);

        props
            .on_download
            .call((items_to_download, selected_folder(), props.request.clone()));
    };

    rsx! {
        div { class: "bg-beet-panel border border-white/10 text-white p-6 sm:p-8 rounded-lg shadow-2xl w-full max-w-2xl mx-auto my-10 font-display relative",
            div { class: "relative mb-6",
                button {
                    class: "absolute left-0 top-1/2 -translate-y-1/2 p-2 rounded-full hover:bg-white/10 transition-colors cursor-pointer",
                    aria_label: "Back",
                    onclick: move |_| props.on_back.call(()),
                    svg {
                        class: "w-5 h-5 text-gray-400",
                        fill: "none",
                        stroke: "currentColor",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            stroke_width: "2",
                            d: "M15 19l-7-7 7-7",
                        }
                    }
                }
                h3 { class: "text-2xl font-bold text-center text-beet-accent", "Download Options" }
            }
            div { class: "mb-4",
                label {
                    r#for: "dl_folder",
                    class: "block text-sm font-medium mb-1 text-gray-400 font-mono",
                    "Select Target Folder"
                }
                select {
                    name: "dl_folder",
                    class: FIELD_CLASS,
                    value: "{selected_folder}",
                    onchange: move |e| selected_folder.set(e.value()),
                    option { value: "", disabled: true, "Select a folder" }
                    for folder in folders.read().iter() {
                        option { value: "{folder.path}", "{folder.name}" }
                    }
                }
            }

            div { class: "space-y-4",
                if !selected_release.label.is_empty() {
                    div { class: "bg-beet-accent/10 border border-beet-accent/40 rounded-md p-3",
                        span { class: "text-[10px] font-mono uppercase tracking-widest text-beet-accent",
                            "Selected release"
                        }
                        h4 { class: "text-md font-bold text-white break-words", "{selected_release.label}" }
                        if let Some(artist) = selected_release.artist.as_ref() {
                            if !artist.is_empty() {
                                p { class: "text-xs text-gray-400 font-mono break-words", "{artist}" }
                            }
                        }
                        if !selected_release_meta.is_empty() {
                            p { class: "text-xs text-gray-500 font-mono break-words", "{selected_release_meta}" }
                        }
                        if let Some(track_count) = selected_release.track_count {
                            p { class: "text-xs text-gray-500 font-mono", "{track_count} tracks" }
                        }
                    }
                }
                if props.is_searching {
                    div { class: "flex flex-col items-center justify-center p-4 bg-white/5 rounded-lg",
                        div { class: "animate-spin rounded-full h-8 w-8 border-t-2 border-b-2 border-beet-accent mb-2" }
                        p { class: "text-sm text-gray-300 animate-pulse text-center font-mono",
                            "Searching... The rarer your track is, the longer the search can take."
                        }
                    }
                } else if results.is_empty() {
                    div { class: "text-center text-gray-500 py-8 font-mono", "No results found" }
                }
                for (idx, album) in results.iter().enumerate() {
                    AlbumResultItem {
                        album: album.clone(),
                        selected_tracks,
                        on_album_select_all: handle_album_select_all,
                        on_track_toggle: handle_track_toggle,
                        on_download_group: handle_download_group,
                        request: item_request.clone(),
                        can_download: !selected_folder.read().is_empty() && !*is_downloading.read(),
                        is_best: idx == 0,
                        starts_expanded: idx == 0,
                    }
                }
            }
            div { class: "fixed bottom-8 right-8",
                button {
                    class: "bg-beet-accent hover:bg-fuchsia-400 text-white font-bold p-4 rounded-full shadow-[0_0_15px_rgba(255,0,255,0.5)] transition-transform hover:scale-105 disabled:bg-gray-600 disabled:cursor-not-allowed disabled:shadow-none flex items-center justify-center cursor-pointer",
                    aria_label: "Start download",
                    disabled: selected_tracks.read().is_empty() || selected_folder.read().is_empty() || *is_downloading.read(),
                    onclick: handle_download,
                    if *is_downloading.read() {
                        // Show spinner when downloading
                        div { class: "animate-spin rounded-full h-6 w-6 border-t-2 border-b-2 border-white" }
                    } else {
                        svg {
                            class: "w-6 h-6",
                            fill: "none",
                            stroke: "currentColor",
                            view_box: "0 0 24 24",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                stroke_width: "2",
                                d: "M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4",
                            }
                        }
                    }
                }
            }
        }
    }
}
