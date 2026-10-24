use api::models::folder::Folder;
use dioxus::prelude::*;
use shared::library::{DownloadActivity, LibraryMatchInfo};
use shared::metadata::{Album, Track};
use std::collections::HashMap;

use super::album::AlbumResult;
use super::download_icon::DownloadRowState;

#[derive(Props, PartialEq, Clone)]
pub struct Props {
    pub editions: Vec<Album>,
    pub selected: usize,
    #[props(default)]
    pub library_match: Option<LibraryMatchInfo>,
    #[props(default)]
    pub request_state: Option<String>,
    #[props(into)]
    pub on_select: EventHandler<usize>,
    pub is_expanded: bool,
    pub on_toggle: Callback,
    pub on_search_sources: Callback,
    pub download_state: DownloadRowState,
    pub folders: Vec<Folder>,
    pub selected_folder_id: Option<String>,
    pub active_menu: Signal<Option<String>>,
    pub tracks: Option<Vec<Track>>,
    pub download_states: Signal<HashMap<String, DownloadRowState>>,
    pub activity: Signal<DownloadActivity>,
    #[props(into)]
    pub on_download: EventHandler<()>,
    #[props(into)]
    pub on_override_download: EventHandler<Folder>,
    #[props(into)]
    pub on_track_download: EventHandler<Track>,
    #[props(into)]
    pub on_track_override_download: EventHandler<(Track, Folder)>,
}

fn edition_label(edition: &Album) -> String {
    let mut parts = vec![edition.title.clone()];
    parts.extend(
        [
            edition.country.clone(),
            edition.medium_label(),
            edition.release_date.clone(),
        ]
        .into_iter()
        .flatten(),
    );
    parts.join(" · ")
}

#[component]
pub fn AlbumGroupResult(props: Props) -> Element {
    let selected = props.selected.min(props.editions.len().saturating_sub(1));
    let Some(album) = props.editions.get(selected).cloned() else {
        return rsx! {};
    };

    rsx! {
      div { class: "space-y-1.5",
        if props.editions.len() > 1 {
          div { class: "flex items-center gap-2 pl-1",
            span { class: "text-[10px] font-mono uppercase tracking-widest text-gray-500 shrink-0",
              "{props.editions.len()} editions"
            }
            select {
              class: "flex-grow min-w-0 bg-beet-dark border border-white/10 rounded px-2 py-1 text-xs font-mono text-gray-300 focus:border-beet-accent focus:outline-none",
              value: "{selected}",
              onchange: move |event| {
                  if let Ok(index) = event.value().parse::<usize>() {
                      props.on_select.call(index);
                  }
              },
              for (index, edition) in props.editions.iter().enumerate() {
                option {
                  value: "{index}",
                  selected: index == selected,
                  "{edition_label(edition)}"
                }
              }
            }
          }
        }

        AlbumResult {
          album,
          library_match: props.library_match.clone(),
          request_state: props.request_state.clone(),
          is_expanded: props.is_expanded,
          on_toggle: props.on_toggle,
          on_search_sources: props.on_search_sources,
          download_state: props.download_state.clone(),
          folders: props.folders.clone(),
          selected_folder_id: props.selected_folder_id.clone(),
          active_menu: props.active_menu,
          tracks: props.tracks.clone(),
          download_states: props.download_states,
          activity: props.activity,
          on_download: props.on_download,
          on_override_download: props.on_override_download,
          on_track_download: props.on_track_download,
          on_track_override_download: props.on_track_override_download,
        }
      }
    }
}
