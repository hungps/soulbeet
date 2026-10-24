use std::collections::{HashMap, HashSet};

use dioxus::logger::tracing::warn;
use dioxus::prelude::*;
use shared::download::{DownloadProgress, DownloadRequestSummary, DownloadState};

mod item;
use api::{CancelDownloadRequest, RequestAction};
use item::DownloadItem;

use crate::use_auth;

#[derive(Props, Clone, PartialEq)]
pub struct DownloadsProps {
    pub is_open: Signal<bool>,
    pub downloads: Signal<HashMap<String, DownloadProgress>>,
}

#[derive(Clone, PartialEq)]
struct TransferGroup {
    id: Option<String>,
    label: String,
    tracks: Vec<DownloadProgress>,
}

fn display_name(item: &str) -> String {
    let normalized = item.replace('\\', "/");
    normalized
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(item)
        .to_string()
}

fn group_tracks(downloads: &HashMap<String, DownloadProgress>) -> Vec<TransferGroup> {
    let mut groups: HashMap<String, TransferGroup> = HashMap::new();

    for file in downloads.values() {
        let key = file
            .batch_id
            .clone()
            .unwrap_or_else(|| format!("single:{}", file.item));
        let label = file
            .batch_label
            .clone()
            .unwrap_or_else(|| display_name(&file.item));
        let group = groups.entry(key).or_insert_with(|| TransferGroup {
            id: file.batch_id.clone(),
            label,
            tracks: Vec::new(),
        });
        group.tracks.push(file.clone());
    }

    let mut groups: Vec<TransferGroup> = groups.into_values().collect();
    for group in &mut groups {
        group
            .tracks
            .sort_by(|a, b| display_name(&a.item).cmp(&display_name(&b.item)));
    }
    groups.sort_by(|a, b| {
        group_rank(a)
            .cmp(&group_rank(b))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    groups
}

fn group_rank(group: &TransferGroup) -> u8 {
    if group.tracks.iter().any(|t| {
        matches!(
            t.state,
            DownloadState::Queued
                | DownloadState::InProgress
                | DownloadState::Importing
                | DownloadState::Completed
        )
    }) {
        0
    } else if group
        .tracks
        .iter()
        .any(|t| matches!(t.state, DownloadState::Stopped | DownloadState::Cancelled))
    {
        1
    } else if group
        .tracks
        .iter()
        .any(|t| matches!(t.state, DownloadState::Failed(_)))
    {
        2
    } else {
        3
    }
}

fn has_active(group: &TransferGroup) -> bool {
    group.tracks.iter().any(|t| {
        matches!(
            t.state,
            DownloadState::Queued
                | DownloadState::InProgress
                | DownloadState::Importing
                | DownloadState::Completed
        )
    })
}

fn has_stopped(group: &TransferGroup) -> bool {
    group
        .tracks
        .iter()
        .any(|t| matches!(t.state, DownloadState::Stopped | DownloadState::Cancelled))
}

fn has_failed(group: &TransferGroup) -> bool {
    group
        .tracks
        .iter()
        .any(|t| matches!(t.state, DownloadState::Failed(_)))
}

fn finished_count(group: &TransferGroup) -> usize {
    group
        .tracks
        .iter()
        .filter(|t| t.state.is_terminal())
        .count()
}

fn group_percent(group: &TransferGroup) -> f64 {
    let total: u64 = group.tracks.iter().map(|t| t.size).sum();
    if total == 0 {
        let sum: f64 = group.tracks.iter().map(|t| t.percent).sum();
        return sum / group.tracks.len() as f64;
    }
    let transferred: u64 = group.tracks.iter().map(|t| t.transferred).sum();
    (transferred as f64 / total as f64) * 100.0
}

#[component]
pub fn Downloads(mut props: DownloadsProps) -> Element {
    let auth = use_auth();
    let mut expanded = use_signal(HashSet::<String>::new);

    use_future(move || async move {
        if !auth.is_logged_in() {
            return;
        }
        match auth.call(api::list_download_requests()).await {
            Ok(requests) => {
                let mut map = props.downloads.write();
                for request in requests {
                    seed_request_tracks(&request, &mut map);
                }
            }
            Err(e) => warn!("Failed to load download requests: {:?}", e),
        }
    });

    let active_downloads: Vec<DownloadProgress> =
        props.downloads.read().values().cloned().collect();
    let groups = group_tracks(&props.downloads.read());

    let processing_count = active_downloads
        .iter()
        .filter(|f| {
            matches!(
                f.state,
                DownloadState::Queued
                    | DownloadState::InProgress
                    | DownloadState::Importing
                    | DownloadState::Completed
            )
        })
        .count();

    let errored_count = active_downloads
        .iter()
        .filter(|f| {
            matches!(
                f.state,
                DownloadState::Failed(_) | DownloadState::Cancelled | DownloadState::Stopped
            )
        })
        .count();

    let clear_finished = move |_| {
        spawn(async move {
            let _ = auth.call(api::dismiss_finished_download_requests()).await;
        });
        let mut map = props.downloads.write();
        map.retain(|_, file| {
            matches!(
                file.state,
                DownloadState::Queued
                    | DownloadState::InProgress
                    | DownloadState::Importing
                    | DownloadState::Completed
                    | DownloadState::Stopped
            )
        });
    };

    let mut downloads_signal = props.downloads;
    let cancel_download = move |file: DownloadProgress| {
        let req = CancelDownloadRequest {
            id: file.id.clone(),
            source: file.source.clone(),
            item: file.item.clone(),
            backend: file.backend.clone(),
        };
        let item_key = file.item.clone();
        spawn(async move {
            if api::cancel_download(req).await.is_ok() {
                let mut map = downloads_signal.write();
                if let Some(entry) = map.get_mut(&item_key) {
                    entry.state = DownloadState::Cancelled;
                }
            }
        });
    };

    let close_modal = move |_| props.is_open.set(false);

    let (modal_opacity, panel_translate, pointer_events) = if (*props.is_open)() {
        ("opacity-100", "translate-x-0", "pointer-events-auto")
    } else {
        ("opacity-0", "translate-x-full", "pointer-events-none")
    };

    rsx! {
      div { class: "fixed inset-0 z-50 flex justify-end transition-opacity duration-200 ease-out {modal_opacity} {pointer_events}",
        // Backdrop
        div {
          class: "absolute inset-0 bg-black/60 backdrop-blur-sm cursor-pointer",
          onclick: close_modal,
        }

        // Panel
        div { class: "relative w-full max-w-md bg-beet-panel border-l border-white/10 h-full shadow-2xl transform transition-transform duration-200 ease-out flex flex-col {panel_translate}",
          //Header
          div { class: "p-6 border-b border-white/10 flex justify-between items-center bg-black/20",
            div {
              h3 { class: "text-xl font-bold text-white font-display",
                "Active Transfers"
              }
              p { class: "text-xs text-beet-leaf font-mono mt-1",
                ":: {processing_count} PROCESSING // {errored_count} ERRORED"
              }
            }
            button {
              class: "text-gray-400 hover:text-white transition-colors cursor-pointer",
              aria_label: "Close downloads",
              onclick: close_modal,
              svg {
                class: "w-6 h-6",
                fill: "none",
                stroke: "currentColor",
                view_box: "0 0 24 24",
                path {
                  stroke_linecap: "round",
                  stroke_linejoin: "round",
                  stroke_width: "2",
                  d: "M6 18L18 6M6 6l12 12",
                }
              }
            }
          }

          // Content
          div { class: "flex-1 overflow-y-auto p-6 no-scrollbar space-y-3",
            if groups.is_empty() {
              div { class: "text-center text-gray-500 py-10 font-mono text-sm",
                "No active transfers in the queue."
              }
            }

            for group in groups.iter() {
              {
                let group_id = group.id.clone();
                let key = group_id.clone().unwrap_or_else(|| format!("single:{}", group.label));
                let is_expanded = expanded.read().contains(&key);
                let is_persisted = group_id.is_some();
                let percent = group_percent(group) as i32;
                let finished = finished_count(group);
                let total = group.tracks.len();
                let active = has_active(group);
                let stopped = has_stopped(group);
                let failed = has_failed(group);
                let stop_id = group_id.clone();
                let resume_id = group_id.clone();
                let retry_id = group_id.clone();
                let remove_id = group_id.clone();

                rsx! {
                  div { key: "{key}", class: "bg-white/5 border border-white/5 rounded-lg overflow-hidden",
                    div {
                      class: "p-3 cursor-pointer hover:border-beet-accent/40 transition-colors",
                      onclick: {
                        let key = key.clone();
                        move |_| {
                            if expanded.read().contains(&key) {
                                expanded.write().remove(&key);
                            } else {
                                expanded.write().insert(key.clone());
                            }
                        }
                      },
                      div { class: "flex items-start justify-between gap-2",
                        div { class: "min-w-0 flex-grow",
                          h4 { class: "text-sm font-bold text-beet-leaf truncate", "{group.label}" }
                          p { class: "text-[10px] font-mono text-gray-500 uppercase tracking-widest",
                            "{finished}/{total} tracks"
                          }
                        }
                        div { class: "flex items-center gap-1 shrink-0",
                          if is_persisted && active {
                            {
                              let request_id = stop_id.clone().unwrap_or_default();
                              rsx! {
                                button {
                                  class: "p-1.5 rounded-md transition-colors cursor-pointer text-orange-300 hover:bg-orange-500/10",
                                  title: "Stop request",
                                  aria_label: "Stop request",
                                  onclick: move |evt: MouseEvent| {
                                      evt.stop_propagation();
                                      let request_id = request_id.clone();
                                      spawn(async move {
                                          let _ = auth
                                              .call(api::stop_download_request(RequestAction { request_id }))
                                              .await;
                                      });
                                  },
                                  svg {
                                    class: "w-4 h-4",
                                    fill: "none",
                                    stroke: "currentColor",
                                    stroke_width: "2",
                                    view_box: "0 0 24 24",
                                    path { stroke_linecap: "round", stroke_linejoin: "round", d: "M6 6h12v12H6z" }
                                  }
                                }
                              }
                            }
                          }
                          if is_persisted && !active && stopped {
                            {
                              let request_id = resume_id.clone().unwrap_or_default();
                              rsx! {
                                button {
                                  class: "p-1.5 rounded-md transition-colors cursor-pointer text-beet-leaf hover:bg-beet-leaf/10",
                                  title: "Resume request",
                                  aria_label: "Resume request",
                                  onclick: move |evt: MouseEvent| {
                                      evt.stop_propagation();
                                      let request_id = request_id.clone();
                                      spawn(async move {
                                          let _ = auth
                                              .call(api::resume_download_request(RequestAction { request_id }))
                                              .await;
                                      });
                                  },
                                  svg {
                                    class: "w-4 h-4",
                                    fill: "currentColor",
                                    stroke: "currentColor",
                                    stroke_width: "1",
                                    view_box: "0 0 24 24",
                                    path { d: "M8 5v14l11-7z" }
                                  }
                                }
                              }
                            }
                          }
                          if is_persisted && failed {
                            {
                              let request_id = retry_id.clone().unwrap_or_default();
                              rsx! {
                                button {
                                  class: "p-1.5 rounded-md transition-colors cursor-pointer text-blue-300 hover:bg-blue-500/10",
                                  title: "Retry failed tracks",
                                  aria_label: "Retry failed tracks",
                                  onclick: move |evt: MouseEvent| {
                                      evt.stop_propagation();
                                      let request_id = request_id.clone();
                                      spawn(async move {
                                          let _ = auth
                                              .call(api::retry_download_request(RequestAction { request_id }))
                                              .await;
                                      });
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
                                      d: "M4 4v6h6M20 20v-6h-6M4 10a8 8 0 0114-4M20 14a8 8 0 01-14 4",
                                    }
                                  }
                                }
                              }
                            }
                          }
                          if is_persisted {
                            {
                              let request_id = remove_id.clone().unwrap_or_default();
                              rsx! {
                                button {
                                  class: "p-1.5 rounded-md transition-colors cursor-pointer text-gray-500 hover:bg-red-500/10 hover:text-red-400",
                                  title: "Remove request",
                                  aria_label: "Remove request",
                                  onclick: move |evt: MouseEvent| {
                                      evt.stop_propagation();
                                      let request_id = request_id.clone();
                                      let mut downloads = props.downloads;
                                      spawn(async move {
                                          if auth
                                              .call(api::remove_download_request(RequestAction {
                                                  request_id: request_id.clone(),
                                              }))
                                              .await
                                              .is_ok()
                                          {
                                              downloads
                                                  .write()
                                                  .retain(|_, file| file.batch_id.as_deref() != Some(request_id.as_str()));
                                          }
                                      });
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
                                      d: "M6 7h12M9 7V4h6v3m-8 0 1 13h8l1-13",
                                    }
                                  }
                                }
                              }
                            }
                          }
                          svg {
                            class: "w-4 h-4 transition-transform duration-200 text-gray-500",
                            class: if is_expanded { "rotate-90" } else { "" },
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
                      }
                      div { class: "mt-2 h-1.5 w-full bg-gray-800 rounded-full overflow-hidden",
                        div {
                          class: "h-full bg-beet-accent transition-all duration-300",
                          style: "width: {percent}%",
                        }
                      }
                    }

                    if is_expanded {
                      div { class: "px-3 pb-3 space-y-2",
                        for file in group.tracks.iter() {
                          DownloadItem { file: file.clone(), on_cancel: cancel_download }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
          // Footer
          div { class: "p-4 border-t border-white/10 bg-black/20",
            button {
              class: "w-full py-2 text-xs font-mono uppercase tracking-widest text-center border border-white/10 hover:bg-white/5 text-gray-400 hover:text-white transition-colors cursor-pointer hover:border-red-500/30",
              onclick: clear_finished,
              "CLEAR COMPLETED"
            }
          }
        }
      }
    }
}

fn seed_request_tracks(
    request: &DownloadRequestSummary,
    map: &mut HashMap<String, DownloadProgress>,
) {
    for track in &request.tracks {
        if map.contains_key(&track.item) {
            continue;
        }
        map.insert(
            track.item.clone(),
            DownloadProgress {
                id: track.transfer_id.clone().unwrap_or_default(),
                source: track.source.clone(),
                item: track.item.clone(),
                size: track.size,
                transferred: track.transferred,
                state: track.state.clone(),
                percent: track.percent,
                speed: track.speed,
                error: track.error.clone(),
                backend: Some("soulseek".to_string()),
                batch_id: Some(request.id.clone()),
                batch_label: Some(request.label.clone()),
            },
        );
    }
}
