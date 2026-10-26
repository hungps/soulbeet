use dioxus::prelude::*;
use shared::download::{DownloadReleaseInfo, DownloadTagApplyRequest, DownloadTagPreviewRequest};
use shared::library::{LibraryTagApplyTrack, LibraryTagPreview};

use crate::components::dashboard::library::suggest_track;
use crate::{friendly_error, use_auth, Modal};

/// Picks the release a kept download belongs to, maps each downloaded file
/// onto a track, and copies them into the library tagged that way.
#[component]
pub fn DownloadTagDialog(
    item: String,
    #[props(default)] release: DownloadReleaseInfo,
    on_close: EventHandler<()>,
) -> Element {
    let auth = use_auth();
    let mut loading = use_signal(|| true);
    let mut preview = use_signal::<Option<LibraryTagPreview>>(|| None);
    let mut candidate = use_signal(|| 0usize);
    let mut selections = use_signal(Vec::<Option<usize>>::new);
    let mut notice = use_signal(|| None::<String>);
    let mut applying = use_signal(|| false);
    // The dialog is keyed by item, so this copy outlives the load future.
    let apply_item = item.clone();

    use_future(move || {
        let item = item.clone();
        let release_mbid = release.release_mbid.clone();
        async move {            match auth
                .call(api::preview_download_tagging(DownloadTagPreviewRequest {
                    item,
                    release_mbid,
                }))
                .await
            {
                Ok(loaded) => {
                    let chosen = loaded
                        .files
                        .iter()
                        .map(|file| file.selected)
                        .collect::<Vec<_>>();
                    candidate.set(loaded.selected_candidate);
                    selections.set(chosen);
                    preview.set(Some(loaded));
                }
                Err(error) => notice.set(Some(friendly_error(&error))),
            }
            loading.set(false);
        }
    });

    let mut select_candidate = move |index: usize| {
        candidate.set(index);
        let Some(loaded) = preview.peek().clone() else {
            return;
        };
        let Some(edition) = loaded.candidates.get(index) else {
            return;
        };
        let chosen = loaded
            .files
            .iter()
            .map(|file| suggest_track(&file.current_title, file.track_number, &edition.tracks))
            .collect();
        selections.set(chosen);
    };

    let apply = move |_| {
        if applying() {
            return;
        }
        let Some(loaded) = preview.peek().clone() else {
            return;
        };
        let Some(edition) = loaded.candidates.get(*candidate.peek()) else {
            return;
        };
        let chosen = selections.peek().clone();
        let tracks: Vec<LibraryTagApplyTrack> = loaded
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                let track = edition.tracks.get(chosen.get(index).copied().flatten()?)?;
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
                    item: file.item.clone(),
                })
            })
            .collect();
        if tracks.is_empty() {
            notice.set(Some("Select at least one track to import".to_string()));
            return;
        }
        applying.set(true);
        notice.set(None);
        let item = apply_item.clone();
        spawn(async move {
            match auth
                .call(api::apply_download_tagging(DownloadTagApplyRequest {
                    item,
                    tracks,
                }))
                .await
            {
                Ok(_) => on_close.call(()),
                Err(error) => {
                    notice.set(Some(friendly_error(&error)));
                    applying.set(false);
                }
            }
        });
    };

    let loaded = preview();
    let selected = candidate().min(
        loaded
            .as_ref()
            .map(|preview| preview.candidates.len().saturating_sub(1))
            .unwrap_or(0),
    );
    let edition = loaded
        .as_ref()
        .and_then(|preview| preview.candidates.get(selected))
        .cloned();

    rsx! {
        Modal {
            on_close: move |_| on_close.call(()),
            header: rsx! {
                div {
                    h2 { class: "text-lg font-bold text-white font-display", "Import into the library" }
                    p { class: "text-[10px] font-mono text-gray-500 uppercase tracking-widest",
                        "Pick the matching release, then confirm each file"
                    }
                }
            },
            if loading() {
                p { class: "text-sm font-mono text-gray-400 py-6 text-center", "Looking for releases…" }
            } else if let Some(loaded) = loaded {
                if loaded.candidates.is_empty() {
                    p { class: "text-sm font-mono text-gray-400 py-6 text-center",
                        "No metadata candidates found for this download."
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
                                for (index, option) in loaded.candidates.iter().enumerate() {
                                    option {
                                        value: "{index}",
                                        selected: index == selected,
                                        "{option.artist} — {option.title} ({option.track_count})"
                                    }
                                }
                            }
                        }

                        if let Some(edition) = edition {
                            div { class: "space-y-1 rounded border border-white/5 bg-beet-dark/40 p-2",
                                for (index, file) in loaded.files.iter().enumerate() {
                                    {
                                        let chosen = selections.read().get(index).copied().flatten();
                                        rsx! {
                                            div { class: "p-1.5 rounded hover:bg-white/5 space-y-1.5",
                                                p { class: "text-xs font-mono text-gray-300 truncate", "{file.name}" }
                                                div { class: "flex items-center gap-2 min-w-0",
                                                    span { class: "text-gray-600 font-mono shrink-0", ">>>" }
                                                    select {
                                                        class: "flex-1 min-w-0 bg-beet-panel border border-white/10 rounded px-2 py-1 text-xs font-mono text-gray-300 focus:border-beet-accent focus:outline-none",
                                                        value: "{chosen.map(|v| v.to_string()).unwrap_or_default()}",
                                                        onchange: move |event| {
                                                            let value = event.value();
                                                            let mut selections = selections.write();
                                                            if selections.len() <= index {
                                                                selections.resize(index + 1, None);
                                                            }
                                                            selections[index] = value.parse::<usize>().ok();
                                                        },
                                                        option { value: "", "— skip —" }
                                                        for (track_index, track) in edition.tracks.iter().enumerate() {
                                                            option {
                                                                value: "{track_index}",
                                                                selected: chosen == Some(track_index),
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

                        if let Some(notice) = notice() {
                            p { class: "text-xs font-mono text-red-400 break-words", "{notice}" }
                        }

                        div { class: "flex justify-end gap-2 pt-1",
                            button {
                                class: "px-3 py-1.5 text-sm font-mono text-gray-400 hover:text-white transition-colors cursor-pointer",
                                onclick: move |_| on_close.call(()),
                                "Cancel"
                            }
                            button {
                                class: "px-3 py-1.5 text-sm font-mono rounded bg-beet-leaf/20 text-beet-leaf hover:bg-beet-leaf/30 transition-colors cursor-pointer disabled:opacity-40 disabled:pointer-events-none",
                                disabled: applying(),
                                onclick: apply,
                                if applying() { "Importing…" } else { "Import into library" }
                            }
                        }
                    }
                }
            } else if let Some(notice) = notice() {
                p { class: "text-xs font-mono text-red-400 py-6 text-center break-words", "{notice}" }
            }
        }
    }
}
