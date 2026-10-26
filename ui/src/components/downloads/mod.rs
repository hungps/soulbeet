use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use dioxus::logger::tracing::warn;
use dioxus::prelude::*;
use shared::download::{
    DownloadProgress, DownloadReleaseInfo, DownloadRequestSummary, DownloadState, ImportChoice,
    KeptImport, MatchChoices, PendingRequestSummary,
};

mod item;
mod picker;
use api::{CancelDownloadRequest, RequestAction};
use item::{shows_choices, shows_retry, DownloadItem, ImportActions};

use crate::{friendly_error, use_auth};

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
                | DownloadState::ImportQueued(_)
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

/// The row whose kept import job the album's own actions use: every row of a
/// batch shares one job, so one row speaks for all of them. Actions move to the
/// album once its transfers are done and at least one row was kept.
fn group_import_row(group: &TransferGroup) -> Option<&DownloadProgress> {
    if group.tracks.len() < 2 || !group.tracks.iter().all(|t| t.state.is_terminal()) {
        return None;
    }
    group.tracks.iter().find(|t| shows_retry(t))
}

fn track_progress_rank(state: &DownloadState) -> Option<u8> {
    match state {
        DownloadState::Queued => Some(0),
        DownloadState::InProgress => Some(1),
        DownloadState::Completed => Some(2),
        DownloadState::ImportQueued(_) | DownloadState::Importing => Some(3),
        DownloadState::Imported => Some(4),
        _ => None,
    }
}

fn group_status(group: &TransferGroup) -> (&'static str, &'static str) {
    let any = |state: &DownloadState| group.tracks.iter().any(|track| &track.state == state);
    if group
        .tracks
        .iter()
        .any(|track| matches!(track.state, DownloadState::Failed(_)))
    {
        return ("FAILED", "bg-red-500/20 text-red-300 border-red-500/40");
    }
    if any(&DownloadState::Stopped) || any(&DownloadState::Cancelled) {
        return (
            "STOPPED",
            "bg-orange-500/20 text-orange-300 border-orange-500/40",
        );
    }
    if any(&DownloadState::ImportSkipped) {
        return (
            "SKIPPED",
            "bg-yellow-500/20 text-yellow-300 border-yellow-500/40",
        );
    }
    let all_queued = !group.tracks.is_empty()
        && group
            .tracks
            .iter()
            .all(|track| matches!(track.state, DownloadState::Queued));
    if all_queued {
        return ("QUEUED", "bg-gray-600/20 text-gray-400 border-gray-500/40");
    }
    let Some(rank) = group
        .tracks
        .iter()
        .filter(|track| !matches!(track.state, DownloadState::Queued))
        .filter_map(|track| track_progress_rank(&track.state))
        .min()
    else {
        return ("PENDING", "bg-gray-600/20 text-gray-400 border-gray-500/40");
    };
    match rank {
        0 => ("QUEUED", "bg-gray-600/20 text-gray-400 border-gray-500/40"),
        1 => (
            "DOWNLOADING",
            "bg-blue-500/20 text-blue-300 border-blue-500/40",
        ),
        2 => (
            "DOWNLOADED",
            "bg-blue-500/20 text-blue-300 border-blue-500/40",
        ),
        3 => (
            "TAGGING",
            "bg-beet-leaf/20 text-beet-leaf border-beet-leaf/40",
        ),
        _ => (
            "TAGGED",
            "bg-green-500/20 text-green-300 border-green-500/40",
        ),
    }
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

/// The selected edition's country, medium and date, as the search card shows them.
fn release_facts(release: &DownloadReleaseInfo, track_count: Option<u32>) -> String {
    let media = release.media.as_ref().map(|media| match track_count {
        Some(count) => format!("{media} ({count})"),
        None => media.clone(),
    });
    [release.country.clone(), media, release.release_date.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<String>>()
        .join(" · ")
}

fn query_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn library_href(
    request: Option<&DownloadRequestSummary>,
    label: &str,
    artist: Option<&str>,
) -> String {
    if let Some(mbid) = request
        .and_then(|request| request.release.release_mbid.as_deref())
        .filter(|mbid| !mbid.is_empty())
    {
        return format!("/dashboard?tab=library&release={}", query_encode(mbid));
    }
    let mut href = format!("/dashboard?tab=library&album={}", query_encode(label));
    if let Some(artist) = artist.filter(|artist| !artist.trim().is_empty()) {
        href.push_str(&format!("&artist={}", query_encode(artist)));
    }
    href
}

#[component]
pub fn Downloads(mut props: DownloadsProps) -> Element {
    let auth = use_auth();
    let mut expanded = use_signal(HashSet::<String>::new);
    let mut busy = use_signal(HashSet::<String>::new);
    let mut clearing = use_signal(|| false);
    let mut clearing_failed = use_signal(|| false);
    let mut pending = use_signal::<Vec<PendingRequestSummary>>(Vec::new);
    let mut requests = use_signal::<HashMap<String, DownloadRequestSummary>>(HashMap::new);
    let is_open = props.is_open;
    let mut downloads_signal = props.downloads;

    use_future(move || async move {
        if !auth.is_logged_in() {
            return;
        }
        loop {
            let open = *is_open.peek();
            if open {
                match auth.call(api::list_download_requests()).await {
                    Ok(overview) => {
                        let needs_seed = {
                            let map = downloads_signal.peek();
                            overview.requests.iter().any(|request| {
                                request
                                    .tracks
                                    .iter()
                                    .any(|track| !map.contains_key(&track.item))
                            })
                        };
                        if needs_seed {
                            let mut map = downloads_signal.write();
                            for request in &overview.requests {
                                seed_request_tracks(request, &mut map);
                            }
                        }
                        let by_id: HashMap<String, DownloadRequestSummary> = overview
                            .requests
                            .iter()
                            .map(|request| (request.id.clone(), request.clone()))
                            .collect();
                        if *requests.peek() != by_id {
                            requests.set(by_id);
                        }
                        if *pending.peek() != overview.pending {
                            pending.set(overview.pending);
                        }
                    }
                    Err(e) => warn!("Failed to load download requests: {:?}", e),
                }
            }
            gloo_timers::future::TimeoutFuture::new(if open { 3000 } else { 750 }).await;
        }
    });

    let active_downloads = sorted_rows(&props.downloads.read());
    let groups = group_tracks(&props.downloads.read());
    let requests_by_id = requests.read().clone();

    let processing_count = active_downloads
        .iter()
        .filter(|f| {
            matches!(
                f.state,
                DownloadState::Queued
                    | DownloadState::InProgress
                    | DownloadState::ImportQueued(_)
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

    let pending_count = pending.read().len();
    let summary = if pending_count > 0 {
        format!(
            ":: {processing_count} PROCESSING // {pending_count} PENDING // {errored_count} ERRORED"
        )
    } else {
        format!(":: {processing_count} PROCESSING // {errored_count} ERRORED")
    };

    let clear_finished = move |_| {
        clearing.set(true);
        spawn(async move {
            let _ = auth.call(api::dismiss_finished_download_requests()).await;
            let groups = group_tracks(&props.downloads.read());
            let fully_tagged: HashSet<String> = groups
                .iter()
                .filter(|group| {
                    group.id.is_some()
                        && !group.tracks.is_empty()
                        && group
                            .tracks
                            .iter()
                            .all(|track| matches!(track.state, DownloadState::Imported))
                })
                .filter_map(|group| group.id.clone())
                .collect();
            let mut cleared = Vec::new();
            let mut map = props.downloads.write();
            map.retain(|item, file| match &file.batch_id {
                Some(batch_id) if fully_tagged.contains(batch_id) => {
                    cleared.push(item.clone());
                    false
                }
                _ => true,
            });
            drop(map);
            if !cleared.is_empty() {
                if let Err(e) = api::forget_import_retries(cleared).await {
                    warn!("Could not drop import retries for cleared rows: {e}");
                }
            }
            clearing.set(false);
        });
    };

    let clear_failed = move |_| {
        clearing_failed.set(true);
        spawn(async move {
            if let Ok(removed) = auth.call(api::dismiss_failed_download_requests()).await {
                let removed: HashSet<String> = removed.into_iter().collect();
                let mut cleared = Vec::new();
                let mut map = props.downloads.write();
                map.retain(|item, file| match &file.batch_id {
                    Some(batch_id) if removed.contains(batch_id) => {
                        cleared.push(item.clone());
                        false
                    }
                    _ => true,
                });
                drop(map);
                if !cleared.is_empty() {
                    if let Err(e) = api::forget_import_retries(cleared).await {
                        warn!("Could not drop import retries for cleared rows: {e}");
                    }
                }
            }
            clearing_failed.set(false);
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

    let mut requests_signal = requests;
    let clear_item = move |(request_id, item): (String, String)| {
        spawn(async move {
            if let Err(e) = auth
                .call(api::dismiss_download_track(
                    request_id.clone(),
                    item.clone(),
                ))
                .await
            {
                warn!("Could not clear tagged track: {:?}", e);
                return;
            }
            downloads_signal.write().remove(&item);
            let empty = {
                let mut map = requests_signal.write();
                match map.get_mut(&request_id) {
                    Some(request) => {
                        request.tracks.retain(|track| track.item != item);
                        request.tracks.is_empty()
                    }
                    None => false,
                }
            };
            if empty {
                requests_signal.write().remove(&request_id);
            }
        });
    };

    let mut fail =
        move |item: &str, error: &ServerFnError, from_picker: bool| match downloads_signal
            .write()
            .get_mut(item)
        {
            Some(entry) => show_failure(entry, error, from_picker),
            None => Failure::OnRow,
        };
    let import_again = use_callback(
        move |(item, choice): (String, ImportChoice)| -> Pin<Box<dyn Future<Output = Result<(), Failure>>>> {
            Box::pin(async move {
                let from_picker = sent_by_picker(&choice);
                api::retry_import(item.clone(), choice)
                    .await
                    .map_err(|e| fail(&item, &e, from_picker))
            })
        },
    );
    let load_choices = use_callback(
        move |item: String| -> Pin<Box<dyn Future<Output = Result<MatchChoices, Failure>>>> {
            Box::pin(async move {
                api::import_choices(item.clone())
                    .await
                    .map_err(|e| fail(&item, &e, true))
            })
        },
    );

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
          class: "absolute inset-0 bg-black/70 cursor-pointer",
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
                "{summary}"
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
          div { class: "flex-1 min-h-0 overflow-y-auto overscroll-contain p-6 no-scrollbar space-y-3",
            if groups.is_empty() && pending.read().is_empty() {
              div { class: "text-center text-gray-500 py-10 font-mono text-sm",
                "No active transfers in the queue."
              }
            }

            for entry in pending.read().iter() {
              {
                let is_searching = entry.state == "searching";
                let (status_label, status_class) = if is_searching {
                  ("SEARCHING", "bg-blue-500/20 text-blue-300 border-blue-500/40")
                } else {
                  ("REQUESTED", "bg-orange-500/20 text-orange-300 border-orange-500/40")
                };
                let detail = if is_searching {
                  "Looking for sources".to_string()
                } else if entry.attempts > 0 {
                  format!("Waiting to retry · attempt {}", entry.attempts)
                } else {
                  "Waiting to retry".to_string()
                };
                rsx! {
                  div { key: "{entry.id}", class: "bg-white/5 border border-white/5 rounded-lg p-3",
                    div { class: "flex items-start justify-between gap-2",
                      div { class: "min-w-0 flex-grow",
                        div { class: "flex items-center gap-2 min-w-0",
                          h4 { class: "text-sm font-bold text-beet-leaf break-words min-w-0 flex-1", "{entry.label}" }
                          span {
                            class: "text-[9px] font-mono px-1.5 py-0.5 rounded border uppercase shrink-0 {status_class}",
                            "{status_label}"
                          }
                        }
                        if let Some(artist) = entry.artist.as_ref() {
                          p { class: "text-[10px] font-mono text-gray-500 break-words", "{artist}" }
                        }
                        p { class: "text-[10px] font-mono text-gray-500 uppercase tracking-widest",
                          if entry.track_count > 0 {
                            "{entry.track_count} tracks · {detail}"
                          } else {
                            "{detail}"
                          }
                        }
                      }
                      if is_searching {
                        div { class: "w-4 h-4 mt-0.5 shrink-0 animate-spin rounded-full border-2 border-transparent border-t-beet-accent border-b-beet-accent" }
                      }
                      button {
                        class: "p-1.5 rounded-md transition-colors cursor-pointer text-gray-400 hover:text-red-300 hover:bg-red-500/10 shrink-0",
                        title: "Cancel request",
                        aria_label: "Cancel request",
                        onclick: {
                            let id = entry.id.clone();
                            move |_| {
                                let id = id.clone();
                                spawn(async move {
                                    if let Err(e) = auth
                                        .call(api::cancel_pending_request(api::PendingAction { id: id.clone() }))
                                        .await
                                    {
                                        warn!("Could not cancel pending request: {:?}", e);
                                    }
                                    pending.write().retain(|entry| entry.id != id);
                                });
                            }
                        },
                        svg {
                          class: "w-4 h-4",
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
                  }
                }
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
                let (status_label, status_class) = group_status(group);
                let is_tagging = group.tracks.iter().all(|track| track.source == "beets");
                let stop_id = group_id.clone();
                let resume_id = group_id.clone();
                let retry_id = group_id.clone();
                let remove_id = group_id.clone();
                let busy_key = group_id.clone().unwrap_or_else(|| key.clone());
                let is_busy = busy.read().contains(&busy_key);
                let stop_busy = busy_key.clone();
                let resume_busy = busy_key.clone();
                let retry_busy = busy_key.clone();
                let remove_busy = busy_key.clone();
                let request = group_id
                    .as_ref()
                    .and_then(|id| requests_by_id.get(id))
                    .cloned();
                let release = request
                    .as_ref()
                    .map(|request| request.release.clone())
                    .unwrap_or_default();
                let group_artist = request.as_ref().and_then(|request| request.artist.clone());
                let group_facts = request
                    .as_ref()
                    .map(|request| release_facts(&request.release, request.track_count))
                    .unwrap_or_default();
                let library_href = library_href(request.as_ref(), &group.label, group_artist.as_deref());

                rsx! {
                  div { key: "{key}", class: "bg-white/5 border border-white/5 rounded-lg overflow-hidden",
                    div {
                      class: "p-3 cursor-pointer hover:border-beet-accent/40 transition-colors",
                      aria_expanded: if is_expanded { "true" } else { "false" },
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
                          div { class: "flex items-start gap-2 min-w-0",
                            a {
                              class: "text-sm font-bold text-beet-leaf break-words min-w-0 flex-1 hover:text-white hover:underline decoration-dotted transition-colors",
                              href: "{library_href}",
                              title: "Open this release in the library",
                              onclick: move |evt: MouseEvent| evt.stop_propagation(),
                              "{group.label}"
                            }
                            span {
                              class: "text-[9px] font-mono px-1.5 py-0.5 rounded border uppercase shrink-0 {status_class}",
                              "{status_label}"
                            }
                          }
                          if let Some(artist) = group_artist.as_ref() {
                            p { class: "text-[10px] font-mono text-gray-400 break-words", "{artist}" }
                          }
                          if !group_facts.is_empty() {
                            p { class: "text-[10px] font-mono text-gray-500 break-words", "{group_facts}" }
                          }
                          p { class: "text-[10px] font-mono text-gray-500 uppercase tracking-widest",
                            "{finished}/{total} tracks"
                          }
                        }
                        div {
                          class: if is_busy { "flex items-center gap-1 shrink-0 pointer-events-none opacity-50" } else { "flex items-center gap-1 shrink-0" },
                          onclick: move |evt: MouseEvent| evt.stop_propagation(),
                          if is_busy {
                            div { class: "w-4 h-4 animate-spin rounded-full border-2 border-transparent border-t-beet-accent border-b-beet-accent" }
                          }
                          if is_persisted && active && !is_tagging {
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
                                      let busy_key = stop_busy.clone();
                                      busy.write().insert(busy_key.clone());
                                      spawn(async move {
                                          let _ = auth
                                              .call(api::stop_download_request(RequestAction { request_id }))
                                              .await;
                                          busy.write().remove(&busy_key);
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
                          if is_persisted && !active && stopped && !is_tagging {
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
                                      let busy_key = resume_busy.clone();
                                      busy.write().insert(busy_key.clone());
                                      spawn(async move {
                                          let _ = auth
                                              .call(api::resume_download_request(RequestAction { request_id }))
                                              .await;
                                          busy.write().remove(&busy_key);
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
                          if is_persisted && failed && !is_tagging {
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
                                      let busy_key = retry_busy.clone();
                                      busy.write().insert(busy_key.clone());
                                      spawn(async move {
                                          let _ = auth
                                              .call(api::retry_download_request(RequestAction { request_id }))
                                              .await;
                                          busy.write().remove(&busy_key);
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
                          if is_persisted && !(is_tagging && active) {
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
                                      let busy_key = remove_busy.clone();
                                      busy.write().insert(busy_key.clone());
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
                                          busy.write().remove(&busy_key);
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
                          class: "h-full bg-beet-accent transition-[width] duration-300",
                          style: "width: {percent}%",
                        }
                      }
                    }

                    if is_expanded {
                      div { class: "px-3 pb-3 space-y-2",
                        if let Some(row) = group_import_row(group) {
                          ImportActions {
                            item: row.item.clone(),
                            state: row.state.clone(),
                            can_choose: shows_choices(row),
                            force: release.clone(),
                            on_import: import_again,
                            on_load_choices: load_choices,
                          }
                        }
                        for file in group.tracks.iter() {
                          {
                            let track = request
                              .as_ref()
                              .and_then(|request| {
                                request.tracks.iter().find(|track| track.item == file.item)
                              })
                              .cloned();
                            rsx! {
                              DownloadItem {
                                file: file.clone(),
                                show_import_actions: group_import_row(group).is_none(),
                                release: release.clone(),
                                track,
                                on_cancel: cancel_download,
                                on_clear: clear_item,
                                on_import: import_again,
                                on_load_choices: load_choices,
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
          // Footer
          div { class: "p-4 border-t border-white/10 bg-black/20",
            div { class: "space-y-2",
              button {
                class: "w-full py-2 text-xs font-mono uppercase tracking-widest text-center border border-white/10 hover:bg-white/5 text-gray-400 hover:text-white transition-colors cursor-pointer hover:border-red-500/30 disabled:opacity-40 disabled:pointer-events-none",
                disabled: clearing(),
                onclick: clear_finished,
                if clearing() { "CLEARING…" } else { "CLEAR COMPLETED" }
              }
              button {
                class: "w-full py-2 text-xs font-mono uppercase tracking-widest text-center border border-white/10 hover:bg-red-900/20 text-gray-500 hover:text-red-300 transition-colors cursor-pointer hover:border-red-500/30 disabled:opacity-40 disabled:pointer-events-none",
                disabled: clearing_failed(),
                onclick: clear_failed,
                if clearing_failed() { "CLEARING…" } else { "CLEAR FAILED" }
              }
            }
          }
        }
      }
    }
}

/// Where a failed request from a row shows why.
#[derive(Debug, Clone, PartialEq)]
enum Failure {
    /// On the row's reason line.
    OnRow,
    /// In the picker that sent it, which stays.
    InPicker(String),
}

/// Picks and match loads come from the picker; Retry and as-is from the row.
fn sent_by_picker(choice: &ImportChoice) -> bool {
    matches!(
        choice,
        ImportChoice::Candidate { .. } | ImportChoice::MusicBrainz(_)
    )
}

/// What a refusal says the server still keeps for the row. None when the
/// request was not refused, or the refusal does not say.
fn kept_after(error: &ServerFnError) -> Option<KeptImport> {
    match error {
        ServerFnError::ServerError {
            code: api::REFUSAL_CODE,
            details: Some(details),
            ..
        } => serde_json::from_value(details.clone()).ok(),
        _ => None,
    }
}

/// Takes away what the server says it no longer keeps for the row, and
/// puts why on the reason line, unless the picker sent the request and
/// stays, which then shows it so the skip reason stays readable.
fn show_failure(row: &mut DownloadProgress, error: &ServerFnError, from_picker: bool) -> Failure {
    match kept_after(error) {
        Some(KeptImport::Gone) => {
            row.retryable = false;
            row.can_choose_import = false;
        }
        Some(KeptImport::WithoutMatches) => row.can_choose_import = false,
        Some(KeptImport::WithMatches) | None => {}
    }
    let message = friendly_error(error);
    if from_picker && row.retryable && row.can_choose_import {
        return Failure::InPicker(message);
    }
    row.error = Some(message);
    Failure::OnRow
}

/// Rows by state, then by item so rows with the same state keep their place.
fn sorted_rows(downloads: &HashMap<String, DownloadProgress>) -> Vec<DownloadProgress> {
    let mut rows: Vec<DownloadProgress> = downloads.values().cloned().collect();
    rows.sort_by(|a, b| {
        a.state
            .partial_cmp(&b.state)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.item.cmp(&b.item))
    });
    rows
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
                local_path: track.local_path.clone(),
                import_output: track.import_output.clone(),
                retryable: track.retryable,
                can_choose_import: track.can_choose_import,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::fullstack::RequestError;

    fn row(item: &str, state: DownloadState) -> (String, DownloadProgress) {
        let p =
            DownloadProgress::queued(item.into(), "peer".into(), item.into(), 1).with_state(state);
        (item.to_string(), p)
    }

    #[test]
    fn equal_states_sort_by_item() {
        let map: HashMap<String, DownloadProgress> = [
            row("c", DownloadState::Imported),
            row("a", DownloadState::Imported),
            row("z", DownloadState::InProgress),
            row("b", DownloadState::Imported),
            row("d", DownloadState::ImportQueued(1)),
        ]
        .into_iter()
        .collect();
        let items: Vec<String> = sorted_rows(&map).into_iter().map(|r| r.item).collect();
        assert_eq!(items, ["z", "d", "a", "b", "c"]);
    }

    fn kept_skip() -> DownloadProgress {
        DownloadProgress {
            error: Some("No close match (2 candidates)".into()),
            retryable: true,
            can_choose_import: true,
            ..row("a", DownloadState::ImportSkipped).1
        }
    }

    fn group_of(rows: Vec<DownloadProgress>) -> TransferGroup {
        TransferGroup {
            id: Some("batch".to_string()),
            label: "An Album".to_string(),
            tracks: rows,
        }
    }

    #[test]
    fn a_finished_album_imports_from_its_own_actions() {
        let skipped = kept_skip();
        let imported = row("b", DownloadState::Imported).1;
        let group = group_of(vec![skipped.clone(), imported.clone()]);
        assert_eq!(group_import_row(&group), Some(&skipped));
    }

    #[test]
    fn a_single_track_keeps_its_own_actions() {
        assert!(group_import_row(&group_of(vec![kept_skip()])).is_none());
    }

    #[test]
    fn a_still_running_album_keeps_the_actions_on_its_rows() {
        let rows = vec![kept_skip(), row("b", DownloadState::InProgress).1];
        assert!(group_import_row(&group_of(rows)).is_none());
    }

    #[test]
    fn an_album_with_nothing_kept_offers_no_actions() {
        let nothing_kept = DownloadProgress {
            retryable: false,
            ..kept_skip()
        };
        let rows = vec![nothing_kept, row("b", DownloadState::Imported).1];
        assert!(group_import_row(&group_of(rows)).is_none());
    }

    /// A refusal as the client gets it: built by the server's helper and
    /// sent through JSON.
    fn refused(message: &str, kept: KeptImport) -> ServerFnError {
        let sent = serde_json::to_value(api::refusal_error_with(message, kept)).unwrap();
        serde_json::from_value(sent).unwrap()
    }

    fn every_choice() -> [ImportChoice; 5] {
        [
            ImportChoice::Auto,
            ImportChoice::AsIs,
            ImportChoice::Candidate {
                data_source: "MusicBrainz".into(),
                id: "r1".into(),
            },
            ImportChoice::MusicBrainz("https://musicbrainz.org/release/x".into()),
            ImportChoice::Force {
                release_mbid: Some("1ccaf444-9043-43f2-b431-72f4c0b94044".into()),
                recording_mbid: Some("11111111-1111-1111-1111-111111111111".into()),
            },
        ]
    }

    #[test]
    fn release_facts_read_country_medium_count_and_date() {
        let release = DownloadReleaseInfo {
            country: Some("SE".into()),
            media: Some("CD".into()),
            release_date: Some("2014-12-01".into()),
            ..Default::default()
        };
        assert_eq!(release_facts(&release, Some(1)), "SE · CD (1) · 2014-12-01");
        assert_eq!(release_facts(&release, None), "SE · CD · 2014-12-01");
        assert_eq!(release_facts(&DownloadReleaseInfo::default(), None), "");
    }

    fn after(choice: &ImportChoice, error: &ServerFnError) -> (DownloadProgress, Failure) {
        let mut row = kept_skip();
        let failure = show_failure(&mut row, error, sent_by_picker(choice));
        (row, failure)
    }

    #[test]
    fn the_server_says_what_a_refusal_keeps() {
        let kept = KeptImport::WithoutMatches;
        assert_eq!(kept_after(&refused("x", kept)), Some(kept));
        let plain = api::refusal_error("Folder not found");
        assert_eq!(kept_after(&plain), None);
    }

    #[test]
    fn a_refusal_that_dropped_the_job_takes_every_action_away() {
        for message in ["Nothing to retry", "The kept files are gone"] {
            for choice in every_choice() {
                let (row, failure) = after(&choice, &refused(message, KeptImport::Gone));
                assert_eq!(failure, Failure::OnRow, "{choice:?}");
                assert_eq!(row.error.as_deref(), Some(message));
                assert!(!row.retryable && !row.can_choose_import, "{choice:?}");
            }
        }
    }

    #[test]
    fn a_refusal_without_matches_keeps_only_retry() {
        let message = "This import has no matches to pick from";
        for choice in every_choice() {
            let (row, failure) = after(&choice, &refused(message, KeptImport::WithoutMatches));
            assert_eq!(failure, Failure::OnRow, "{choice:?}");
            assert_eq!(row.error.as_deref(), Some(message));
            assert!(row.retryable && !row.can_choose_import, "{choice:?}");
        }
        // A match load refused the same way takes the picker too.
        let mut row = kept_skip();
        show_failure(
            &mut row,
            &refused(message, KeptImport::WithoutMatches),
            true,
        );
        assert!(row.retryable && !row.can_choose_import);
    }

    #[test]
    fn a_refused_pick_keeps_every_action_and_shows_why_in_the_picker() {
        let text = "Track imports need a MusicBrainz recording, not a release";
        for choice in every_choice().into_iter().filter(sent_by_picker) {
            let (row, failure) = after(&choice, &refused(text, KeptImport::WithMatches));
            assert_eq!(failure, Failure::InPicker(text.into()), "{choice:?}");
            assert_eq!(row.error.as_deref(), Some("No close match (2 candidates)"));
            assert!(row.retryable && row.can_choose_import);
        }
    }

    #[test]
    fn an_unreachable_server_changes_no_action() {
        let error = ServerFnError::Request(RequestError::Request("connection refused".into()));
        let text = "Could not reach the server. Check your connection.";
        let (row, failure) = after(&ImportChoice::Auto, &error);
        assert_eq!(failure, Failure::OnRow);
        assert_eq!(row.error.as_deref(), Some(text));
        assert!(row.retryable && row.can_choose_import);

        // A failed pick or match load keeps the skip reason.
        let mut row = kept_skip();
        assert_eq!(
            show_failure(&mut row, &error, true),
            Failure::InPicker(text.into())
        );
        assert_eq!(row, kept_skip());
    }
}
