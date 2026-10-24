pub mod album;
pub mod context;
pub mod track;

pub use context::{AutoDownloadSignal, SearchPrefill, SearchReset};

mod download_icon;
pub use download_icon::{DownloadIcon, DownloadRowState};

mod download_options_menu;

mod inline_track_panel;
mod inline_track_row;

mod folder_chip;
use folder_chip::FolderChip;

use api::models::folder::Folder;
use dioxus::logger::tracing::{info, warn};
use dioxus::prelude::*;
use shared::download::{
    AutoDownloadEvent, DownloadQuery, DownloadRequestContext, DownloadableGroup, DownloadableItem,
    SearchState as DownloadSearchState,
};
use shared::metadata::{
    Album, AlbumWithTracks, Provider, SearchFilterOptions, SearchFilters, SearchResult,
    SearchResults, Track,
};
use std::collections::{HashMap, HashSet};

use track::TrackResult;

use crate::search::album::AlbumResult;
use crate::settings_context::use_settings;
use crate::{use_auth, use_system_health, Button, SystemStatus};

mod download_results;
use download_results::DownloadResults;

mod toast;
use toast::{FallbackToast, FallbackToastData};

mod search_type_toggle;
use search_type_toggle::{SearchType, SearchTypeToggle};

mod albumgroup;
use albumgroup::AlbumGroupResult;

mod filter_chips;
use filter_chips::FilterChips;

#[component]
pub fn Search() -> Element {
    let auth = use_auth();
    let mut settings = use_settings();
    let mut search_results = use_signal::<Option<SearchResults>>(|| None);
    let mut search = use_signal(String::new);
    let mut artist = use_signal::<Option<String>>(|| None);
    let mut search_type = use_signal(|| settings.last_search_type());
    let mut loading = use_signal(|| false);
    let mut expanded_albums = use_signal::<HashSet<String>>(HashSet::new);
    let mut album_cache = use_signal::<HashMap<String, AlbumWithTracks>>(HashMap::new);
    let mut download_options = use_signal::<Option<Vec<DownloadableGroup>>>(|| None);
    let mut is_downloading = use_signal(|| false);
    let search_reset = try_use_context::<SearchReset>();
    let search_prefill = try_use_context::<SearchPrefill>();

    let system_health = use_system_health();

    // Download state tracking
    let mut download_states = use_signal::<HashMap<String, DownloadRowState>>(HashMap::new);
    let mut batch_to_item = use_signal::<HashMap<String, String>>(HashMap::new);
    let mut folders = use_signal::<Vec<Folder>>(Vec::new);
    let mut selected_folder_id = use_signal::<Option<String>>(|| None);
    let mut fallback_toasts = use_signal::<Vec<FallbackToastData>>(Vec::new);
    let mut batch_to_name = use_signal::<HashMap<String, String>>(HashMap::new);
    let mut active_menu = use_signal::<Option<String>>(|| None);
    let mut filters = use_signal(SearchFilters::default);
    let mut filter_options = use_signal(SearchFilterOptions::default);
    let mut filter_provider = use_signal(String::new);
    let mut selected_edition = use_signal::<HashMap<String, usize>>(HashMap::new);
    let mut download_request_context = use_signal::<Option<DownloadRequestContext>>(|| None);
    let mut selected_provider = use_signal(|| settings.default_provider());
    let mut provider_synced = use_signal(|| false);

    use_effect(move || {
        if settings.is_loaded() && !provider_synced() {
            selected_provider.set(settings.default_provider());
            provider_synced.set(true);
        }
    });

    use_effect(move || {
        let providers = settings.providers();
        if providers.is_empty() {
            return;
        }
        if !providers
            .iter()
            .any(|provider| provider.id == selected_provider())
        {
            selected_provider.set("musicbrainz".to_string());
        }
    });

    use_effect(move || {
        let provider = selected_provider();
        if *filter_provider.peek() == provider {
            return;
        }
        filter_provider.set(provider.clone());
        filter_options.set(SearchFilterOptions::default());
        filters.set(SearchFilters::default());
        spawn(async move {
            match auth
                .call(api::search_filter_options(api::FilterOptionsQuery {
                    provider: Some(provider),
                }))
                .await
            {
                Ok(options) => {
                    filters.set(default_search_filters(&options));
                    filter_options.set(options);
                }
                Err(e) => info!("Failed to load search filters: {:?}", e),
            }
        });
    });

    // Track if we've synced search_type from settings (to avoid saving on initial load)
    let mut synced = use_signal(|| false);

    // Sync search type from settings once loaded
    use_effect(move || {
        if settings.is_loaded() && !synced() {
            search_type.set(settings.last_search_type());
            synced.set(true);
        }
    });

    // Persist search type changes to settings
    use_effect(move || {
        let current_type = search_type();
        if synced() {
            spawn(async move {
                let _ = settings.set_last_search_type(current_type).await;
            });
        }
    });

    // Sync selected_folder_id from settings
    use_effect(move || {
        if settings.is_loaded() {
            if let Some(us) = settings.get() {
                if us.default_download_folder_id.is_some() {
                    selected_folder_id.set(us.default_download_folder_id.clone());
                }
            }
        }
    });

    // Fetch user folders
    use_future(move || async move {
        if let Ok(user_folders) = auth.call(api::get_user_folders()).await {
            if user_folders.len() == 1 {
                selected_folder_id.set(Some(user_folders[0].id.clone()));
            }
            folders.set(user_folders);
        }
    });

    use_effect(move || {
        if let Some(reset) = search_reset {
            if reset.0() > 0 {
                search_results.set(None);
                search.set(String::new());
                artist.set(None);
                expanded_albums.write().clear();
                album_cache.write().clear();
                loading.set(false);
            }
        }
    });

    // Buffer for events that arrive before batch_to_item is populated (race condition)
    let mut pending_events = use_signal::<Vec<AutoDownloadEvent>>(Vec::new);

    // Route AutoDownloadEvents from WebSocket to per-row download states
    let auto_dl = try_use_context::<AutoDownloadSignal>();
    use_effect(move || {
        let Some(mut ctx) = auto_dl else { return };
        // Subscribe only to ctx.0 (WS event slot). Reading other signals with
        // .read() would subscribe to them too, and writing the same signal
        // later in this effect would self-retrigger because Dioxus 0.7
        // Signal::set has no PartialEq dedupe (dioxus-signals/src/write.rs:345).
        let Some(event) = (ctx.0)() else { return };
        (ctx.0).set(None);

        // Collect the new event plus any buffered ones — peek, no subscribe
        let mut events_to_process = pending_events.peek().clone();
        events_to_process.push(event);
        let mut still_pending = Vec::new();

        for event in events_to_process {
            let batch_id = match &event {
                AutoDownloadEvent::Searching { batch_id, .. } => batch_id.clone(),
                AutoDownloadEvent::ScoringResults { batch_id, .. } => batch_id.clone(),
                AutoDownloadEvent::PickedSource { batch_id, .. } => batch_id.clone(),
                AutoDownloadEvent::Downloading { batch_id } => batch_id.clone(),
                AutoDownloadEvent::FallbackToManual { batch_id, .. } => batch_id.clone(),
                AutoDownloadEvent::Failed { batch_id, .. } => batch_id.clone(),
            };

            let item_id = batch_to_item.peek().get(&batch_id).cloned();
            let Some(item_id) = item_id else {
                // batch_id not yet registered, buffer for retry
                still_pending.push(event);
                continue;
            };

            match event {
                AutoDownloadEvent::Searching { .. } | AutoDownloadEvent::ScoringResults { .. } => {
                    download_states.write().insert(item_id.clone(), DownloadRowState::Searching);
                    // D-09: propagate to expanded track rows
                    if let Some(cached) = album_cache.peek().get(&item_id) {
                        let mut states = download_states.write();
                        for track in &cached.tracks {
                            states.insert(track.id.clone(), DownloadRowState::Searching);
                        }
                    }
                }
                AutoDownloadEvent::PickedSource { .. } | AutoDownloadEvent::Downloading { .. } => {
                    let item_id_timer = item_id.clone();
                    download_states.write().insert(item_id.clone(), DownloadRowState::Done);
                    // D-09: propagate to expanded track rows
                    if let Some(cached) = album_cache.peek().get(&item_id) {
                        let mut states = download_states.write();
                        for track in &cached.tracks {
                            states.insert(track.id.clone(), DownloadRowState::Done);
                        }
                    }

                    spawn(async move {
                        gloo_timers::future::TimeoutFuture::new(5000).await;
                        let current = download_states.peek().get(&item_id_timer).cloned();
                        if matches!(current, Some(DownloadRowState::Done)) {
                            download_states.write().insert(item_id_timer, DownloadRowState::Idle);
                        }
                    });
                }
                AutoDownloadEvent::FallbackToManual { batch_id, results, .. } => {
                    download_states.write().insert(item_id.clone(), DownloadRowState::Idle);

                    let track_name = batch_to_name.peek().get(&batch_id).cloned().unwrap_or(item_id.clone());

                    let toast_id = batch_id.clone();
                    fallback_toasts.write().push(FallbackToastData {
                        id: batch_id,
                        track_name,
                        results,
                    });

                    spawn(async move {
                        gloo_timers::future::TimeoutFuture::new(8000).await;
                        fallback_toasts.write().retain(|t| t.id != toast_id);
                    });
                }
                AutoDownloadEvent::Failed { error, .. } => {
                    download_states.write().insert(item_id, DownloadRowState::Failed(error));
                }
            }
        }

        // Safe to always set: neither this effect nor the drain effect
        // below subscribe to `pending_events` (peek throughout), and the
        // render does not read it either, so this is a leaf write.
        pending_events.set(still_pending);
    });

    // Drain pending events when batch_to_item changes (new batch registered).
    // Subscribe ONLY to batch_to_item via the `_trigger` read; every other
    // signal is peeked. Without this, `pending_events.set(...)` below would
    // self-retrigger the effect with the same value (Dioxus 0.7 Signal::set
    // has no PartialEq dedupe) and the race between the WS Searching event
    // and the auto_download HTTP response would freeze the browser.
    use_effect(move || {
        let _trigger = batch_to_item.read().len();
        if pending_events.peek().is_empty() { return; }

        let events = pending_events.peek().clone();
        let mut still_pending = Vec::new();

        for event in events {
            let batch_id = match &event {
                AutoDownloadEvent::Searching { batch_id, .. } => batch_id.clone(),
                AutoDownloadEvent::ScoringResults { batch_id, .. } => batch_id.clone(),
                AutoDownloadEvent::PickedSource { batch_id, .. } => batch_id.clone(),
                AutoDownloadEvent::Downloading { batch_id } => batch_id.clone(),
                AutoDownloadEvent::FallbackToManual { batch_id, .. } => batch_id.clone(),
                AutoDownloadEvent::Failed { batch_id, .. } => batch_id.clone(),
            };

            let item_id = batch_to_item.peek().get(&batch_id).cloned();
            let Some(item_id) = item_id else {
                still_pending.push(event);
                continue;
            };

            match event {
                AutoDownloadEvent::Searching { .. } | AutoDownloadEvent::ScoringResults { .. } => {
                    download_states.write().insert(item_id.clone(), DownloadRowState::Searching);
                    // D-09: propagate to expanded track rows
                    if let Some(cached) = album_cache.peek().get(&item_id) {
                        let mut states = download_states.write();
                        for track in &cached.tracks {
                            states.insert(track.id.clone(), DownloadRowState::Searching);
                        }
                    }
                }
                AutoDownloadEvent::PickedSource { .. } | AutoDownloadEvent::Downloading { .. } => {
                    let item_id_timer = item_id.clone();
                    download_states.write().insert(item_id.clone(), DownloadRowState::Done);
                    // D-09: propagate to expanded track rows
                    if let Some(cached) = album_cache.peek().get(&item_id) {
                        let mut states = download_states.write();
                        for track in &cached.tracks {
                            states.insert(track.id.clone(), DownloadRowState::Done);
                        }
                    }
                    spawn(async move {
                        gloo_timers::future::TimeoutFuture::new(5000).await;
                        let current = download_states.peek().get(&item_id_timer).cloned();
                        if matches!(current, Some(DownloadRowState::Done)) {
                            download_states.write().insert(item_id_timer, DownloadRowState::Idle);
                        }
                    });
                }
                AutoDownloadEvent::FallbackToManual { batch_id, results, .. } => {
                    download_states.write().insert(item_id.clone(), DownloadRowState::Idle);
                    let track_name = batch_to_name.peek().get(&batch_id).cloned().unwrap_or(item_id.clone());
                    let toast_id = batch_id.clone();
                    fallback_toasts.write().push(FallbackToastData {
                        id: batch_id,
                        track_name,
                        results,
                    });
                    spawn(async move {
                        gloo_timers::future::TimeoutFuture::new(8000).await;
                        fallback_toasts.write().retain(|t| t.id != toast_id);
                    });
                }
                AutoDownloadEvent::Failed { error, .. } => {
                    download_states.write().insert(item_id, DownloadRowState::Failed(error));
                }
            }
        }

        pending_events.set(still_pending);
    });

    if !auth.is_logged_in() {
        info!("User not logged in");
        return rsx! {};
    }

    // Start an auto_download for a specific folder
    let mut start_auto_download = move |item_id: String, query: DownloadQuery, folder: Folder| {
        download_request_context.set(Some(query.request_context()));
        download_states.write().insert(item_id.clone(), DownloadRowState::Searching);

        // Propagate to expanded track rows (D-09)
        if expanded_albums.read().contains(&item_id) {
            if let Some(cached) = album_cache.read().get(&item_id) {
                let mut states = download_states.write();
                for track in &cached.tracks {
                    states.insert(track.id.clone(), DownloadRowState::Searching);
                }
            }
        }

        // Extract display name before query is moved into the request
        let display_name = query.album.as_ref().map(|a| a.title.clone())
            .or_else(|| query.tracks.first().map(|t| format!("{} - {}", t.artist, t.title)))
            .unwrap_or_else(|| "Unknown".to_string());

        spawn(async move {
            let result = auth
                .call(api::auto_download(api::AutoDownloadRequest {
                    query,
                    folder_id: folder.id.clone(),
                    folder_path: folder.path.clone(),
                }))
                .await;

            match result {
                Ok(api::AutoDownloadResult::Accepted { batch_id }) => {
                    batch_to_item.write().insert(batch_id.clone(), item_id);
                    batch_to_name.write().insert(batch_id, display_name);
                }
                Ok(api::AutoDownloadResult::Error(e)) => {
                    download_states
                        .write()
                        .insert(item_id, DownloadRowState::Failed(e));
                }
                Err(e) => {
                    download_states
                        .write()
                        .insert(item_id, DownloadRowState::Failed(e.to_string()));
                }
            }
        });
    };

    // Auto-download with default folder resolution
    let mut handle_auto_download = move |item_id: String, query: DownloadQuery| {
        let folder_id = selected_folder_id();
        let folder = folder_id
            .as_ref()
            .and_then(|fid| folders.read().iter().find(|f| f.id == *fid).cloned());

        let Some(folder) = folder else {
            return;
        };

        start_auto_download(item_id, query, folder);
    };

    // Override download to a specific folder (D-02, does not change default)
    let mut handle_override_download =
        move |item_id: String, query: DownloadQuery, folder: Folder| {
            start_auto_download(item_id, query, folder);
        };

    // Handle folder change from FolderChip
    let handle_folder_change = move |folder: Folder| {
        selected_folder_id.set(Some(folder.id.clone()));
        spawn(async move {
            let _ = settings
                .update(api::UpdateUserSettings {
                    default_download_folder_id: Some(folder.id),
                    ..Default::default()
                })
                .await;
        });
    };

    let download = move |query: DownloadQuery| async move {
        download_request_context.set(Some(query.request_context()));
        loading.set(true);
        download_options.set(Some(vec![]));

        let search_id = match auth.call(api::start_download_search(query)).await {
            Ok(id) => id,
            Err(e) => {
                warn!("Failed to start download search: {:?}", e);
                loading.set(false);
                return;
            }
        };

        loop {
            match auth
                .call(api::poll_download_search(api::PollQuery {
                    search_id: search_id.clone(),
                    backend: None,
                }))
                .await
            {
                Ok(response) => {
                    download_options.with_mut(|current| {
                        if let Some(list) = current {
                            for new_group in response.groups {
                                if let Some(pos) = list.iter().position(|x| {
                                    x.source == new_group.source && x.group_id == new_group.group_id
                                }) {
                                    // Safeguard against incomplete albums
                                    list[pos] = new_group;
                                } else {
                                    list.push(new_group);
                                }
                            }

                            // Resort new results by score
                            list.sort_by(|a, b| {
                                b.score
                                    .partial_cmp(&a.score)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            });
                        }
                    });

                    if response.state != DownloadSearchState::InProgress {
                        break;
                    }
                }
                Err(e) => {
                    info!("Failed to poll search: {:?}", e);
                    break;
                }
            }
        }
        loading.set(false);
    };

    let download_tracks = move |(items, folder, request): (
        Vec<DownloadableItem>,
        String,
        DownloadRequestContext,
    )| async move {
        match auth
            .call(api::download(api::DownloadRequest {
                items,
                target_folder: folder,
                backend: None,
                request,
            }))
            .await
        {
            Ok(_res) => info!("Downloads started"),
            Err(e) => warn!("Failed to start downloads: {:?}", e),
        }
        is_downloading.set(false);
    };

    let perform_search = move || async move {
        loading.set(true);
        download_options.set(None);

        let provider = Some(selected_provider());

        let query_data = api::SearchQuery {
            artist: artist(),
            query: search(),
            provider,
            filters: filters(),
        };

        let result = match search_type() {
            SearchType::Album => auth.call(api::search_album(query_data)).await,
            SearchType::Track => auth.call(api::search_track(query_data)).await,
        };

        if let Ok(data) = result {
            search_results.set(Some(data));
        }
        loading.set(false);
    };

    use_effect(move || {
        if let Some(mut prefill) = search_prefill {
            if let Some((prefill_artist, prefill_query)) = (prefill.0)() {
                artist.set(Some(prefill_artist));
                search.set(prefill_query);
                (prefill.0).set(None);
                spawn(perform_search());
            }
        }
    });

    let mut toggle_expand = move |album_id: String, provider: Provider| {
        let is_expanded = expanded_albums.read().contains(&album_id);
        if is_expanded {
            expanded_albums.write().remove(&album_id);
        } else {
            expanded_albums.write().insert(album_id.clone());
            let needs_fetch = !album_cache.read().contains_key(&album_id);
            if needs_fetch {
                let aid = album_id.clone();
                spawn(async move {
                    match auth
                        .call(api::find_album(api::AlbumQuery {
                            id: aid.clone(),
                            provider: Some(provider),
                        }))
                        .await
                    {
                        Ok(album_data) => {
                            album_cache.write().insert(aid, album_data);
                        }
                        Err(e) => {
                            info!("Failed to fetch album details for {}: {:?}", aid, e);
                            expanded_albums.write().remove(&aid);
                        }
                    }
                });
            }
        }
    };

    let mut toggle_filter = move |(field, value): (u8, String)| {
        let mut current = filters();
        let list = match field {
            0 => &mut current.release_types,
            1 => &mut current.secondary_types,
            _ => &mut current.media_formats,
        };
        match list.iter().position(|item| item == &value) {
            Some(index) => {
                list.remove(index);
            }
            None => list.push(value),
        }
        filters.set(current);
        if !search.peek().is_empty() {
            spawn(perform_search());
        }
    };

    let mut handle_track_download = move |track: Track| {
        let folder_id = selected_folder_id();
        let folder = folder_id
            .as_ref()
            .and_then(|fid| folders.read().iter().find(|f| f.id == *fid).cloned());

        let Some(folder) = folder else {
            return;
        };

        start_auto_download(track.id.clone(), DownloadQuery::from(track), folder);
    };

    let mut handle_track_override_download = move |(track, folder): (Track, Folder)| {
        start_auto_download(track.id.clone(), DownloadQuery::from(track), folder);
    };

    let mut handle_provider_change = move |value: String| {
        selected_provider.set(value.clone());
        let mut settings = settings;
        spawn(async move {
            let _ = settings
                .update(api::UpdateUserSettings {
                    default_metadata_provider: Some(value),
                    ..Default::default()
                })
                .await;
        });
    };

    rsx! {
      // bg decorations
      div { class: "fixed top-1/4 -left-10 w-64 h-64 bg-beet-accent/10 rounded-full blur-[150px] pointer-events-none max-md:hidden" }
      div { class: "fixed bottom-1/4 -right-10 w-64 h-64 bg-beet-leaf/10 rounded-full blur-[150px] pointer-events-none max-md:hidden" }

      div {
        class: "w-full max-w-3xl space-y-8 z-10 mx-auto flex flex-col items-center mt-20",
        onclick: move |_| {
            if active_menu().is_some() {
                active_menu.set(None);
            }
        },

        // Title Area
        div { class: "text-center space-y-2",
          h2 { class: "text-4xl md:text-6xl font-bold tracking-tight",
            span { class: "text-white", "Harvest" }
            " "
            span { class: "text-beet-leaf font-light italic", "Music." }
          }
          p { class: "text-gray-400 font-mono text-sm",
            "Search & Download // Manage Your Library"
          }
        }

        // Search bar
        div { class: "w-full relative group",

          div { class: "hidden md:block absolute -inset-1 bg-gradient-to-r from-beet-accent to-beet-leaf rounded-t-lg rounded-b-0 md:rounded-b-lg blur opacity-25 group-hover:opacity-50 transition duration-1000 group-hover:duration-200" }
          div { class: "relative flex items-center bg-beet-dark border border-white/10 rounded-t-lg rounded-b-0 md:rounded-b-lg p-2 shadow-2xl",
            div { class: "pl-4 pr-2 text-gray-500",
              svg {
                class: "w-6 h-6",
                fill: "none",
                stroke: "currentColor",
                view_box: "0 0 24 24",
                path {
                  stroke_linecap: "round",
                  stroke_linejoin: "round",
                  stroke_width: "2",
                  d: "M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z",
                }
              }
            }
            input {
              "type": "text",
              value: "{search}",
              class: "w-2/3 bg-transparent border-none focus:ring-0 text-white text-sm placeholder-gray-600 font-mono h-10 focus:outline-none",
              placeholder: "Search artist, album or track...",
              oninput: move |event| search.set(event.value()),
              onkeydown: move |event| {
                  if event.key() == Key::Enter {
                      spawn(perform_search());
                  }
              },
            }

            div { class: "hidden md:flex h-8 w-px bg-white/10 mx-2" }
            input {
              "type": "text",
              value: "{artist.read().clone().unwrap_or_default()}",
              class: "hidden md:flex w-1/3 bg-transparent border-none focus:ring-0 text-gray-400 text-sm placeholder-gray-700 font-mono h-10 focus:outline-none",
              placeholder: "Artist (opt)",
              oninput: move |event| {
                  let val = event.value();
                  if val.is_empty() { artist.set(None) } else { artist.set(Some(val)) }
              },
              onkeydown: move |event| {
                  if event.key() == Key::Enter {
                      spawn(perform_search());
                  }
              },
            }
            div { class: "hidden md:flex items-center",
              if folders.read().len() > 1 {
                FolderChip {
                  folders: folders.read().clone(),
                  selected_folder_id,
                  on_folder_change: handle_folder_change,
                  active_menu,
                }
              }
              SearchTypeToggle { search_type }
              Button {
                class: "rounded ml-2 whitespace-nowrap",
                disabled: loading() || search.read().is_empty(),
                onclick: move |_| {
                    spawn(perform_search());
                },
                "SEARCH"
              }
            }
          }
          // Mobile search bar
          div { class: "md:hidden relative flex items-center bg-beet-dark border border-t-0 md:border-t border-white/10 rounded-b-lg rounded-t-0 md:rounded-t-lg p-2 shadow-2xl",
            input {
              "type": "text",
              value: "{artist.read().clone().unwrap_or_default()}",
              class: "w-full pl-4 bg-transparent border-none focus:ring-0 text-gray-400 text-sm placeholder-gray-700 font-mono h-10 focus:outline-none",
              placeholder: "Artist (opt)",
              oninput: move |event| {
                  let val = event.value();
                  if val.is_empty() { artist.set(None) } else { artist.set(Some(val)) }
              },
              onkeydown: move |event| {
                  if event.key() == Key::Enter {
                      spawn(perform_search());
                  }
              },
            }

            SearchTypeToggle { search_type }

            Button {
              class: "rounded ml-2 whitespace-nowrap",
              disabled: loading() || search.read().is_empty(),
              onclick: move |_| {
                  spawn(perform_search());
              },
              "SEARCH"
            }
          }
        }

        div { class: "w-full -mt-4 space-y-3",
          div { class: "flex items-center justify-between gap-3 bg-beet-panel/30 border border-white/5 rounded-lg px-3 py-2",
            span { class: "text-[10px] font-mono uppercase tracking-widest text-gray-500 shrink-0",
              "Metadata Provider"
            }
            select {
              class: "flex-grow min-w-0 bg-beet-dark border border-white/10 rounded px-2 py-1 text-xs font-mono text-gray-300 focus:border-beet-accent focus:outline-none",
              aria_label: "Metadata Provider",
              value: "{selected_provider}",
              onchange: move |event| handle_provider_change(event.value()),
              for provider in settings.providers().iter() {
                option {
                  value: "{provider.id}",
                  selected: provider.id == selected_provider(),
                  disabled: !provider.available,
                  if provider.available {
                    "{provider.name}"
                  } else {
                    "{provider.name} (not configured)"
                  }
                }
              }
            }
          }

          if search_type() == SearchType::Album && filter_options.read().is_supported() {
            div { class: "w-full space-y-2.5 bg-beet-panel/30 border border-white/5 rounded-lg p-3",
              FilterChips {
                label: "Type".to_string(),
                values: filter_options.read().release_types.clone(),
                selected: filters.read().release_types.clone(),
                on_toggle: move |value| toggle_filter((0, value)),
              }
              FilterChips {
                label: "Secondary".to_string(),
                values: filter_options.read().secondary_types.clone(),
                selected: filters.read().secondary_types.clone(),
                on_toggle: move |value| toggle_filter((1, value)),
              }
              FilterChips {
                label: "Medium".to_string(),
                values: filter_options.read().media_formats.clone(),
                selected: filters.read().media_formats.clone(),
                on_toggle: move |value| toggle_filter((2, value)),
              }
            }
          }
        }


        SystemStatus {
            health: system_health.get(),
            media_status: auth.media_status(),
            media_server: auth.media_server(),
        }

        // Results
        if let Some(results) = download_options.read().clone() {
          DownloadResults {
            results,
            is_searching: loading(),
            is_downloading,
            request: download_request_context.read().clone().unwrap_or_default(),
            on_download: move |data| {
                spawn(download_tracks(data));
            },
            on_back: move |_| {
                download_options.set(None);
                loading.set(false);
            },
          }
        } else if loading() {
          div { class: "flex flex-col justify-center items-center py-10",
            div { class: "animate-spin rounded-full h-16 w-16 border-t-4 border-b-4 border-beet-accent" }
          }
        } else {
          match &*search_results.read() {
              Some(ref data) if !data.results.is_empty() => {
                let provider = data.provider;
                rsx! {
                  div { class: "w-full bg-beet-panel/50 border border-white/5 p-6 backdrop-blur-sm mt-8 rounded-lg",
                    h5 { class: "text-xl font-display font-bold mb-4 border-b border-white/10 pb-2 text-white",
                      "Search Results"
                    }
                    ul { class: "list-none p-0 space-y-4",
                      for item in data.results.iter() {
                        match item {
                            SearchResult::Track(ref track) => {
                                let track_clone = track.clone();
                                let track_clone_2 = track.clone();
                                let track_for_dl = track.clone();
                                let track_for_override = track.clone();
                                let track_id = track.id.clone();
                                let dl_state = download_states.read().get(&track_id).cloned().unwrap_or_default();
                                let has_folders = !folders.read().is_empty();
                                let effective_state = if !has_folders { DownloadRowState::Disabled } else { dl_state };
                                let current_folders = folders.read().clone();
                                let current_folder_id = selected_folder_id();
                                rsx! {
                                  li { key: "{track.id}",
                                    TrackResult {
                                      on_search_sources: move || {
                                          spawn(download(DownloadQuery::from(track_clone.clone())));
                                      },
                                      on_album_click: {
                                          let album_id_for_expand = track_clone_2.album_id.clone();
                                          move || {
                                              if let Some(ref aid) = album_id_for_expand {
                                                  toggle_expand(aid.clone(), provider);
                                              }
                                          }
                                      },
                                      track: track.clone(),
                                      download_state: effective_state,
                                      folders: current_folders,
                                      selected_folder_id: current_folder_id.clone(),
                                      active_menu,
                                      on_download: move |_| {
                                          handle_auto_download(track_for_dl.id.clone(), DownloadQuery::from(track_for_dl.clone()));
                                      },
                                      on_override_download: move |folder: Folder| {
                                          handle_override_download(
                                              track_for_override.id.clone(),
                                              DownloadQuery::from(track_for_override.clone()),
                                              folder,
                                          );
                                      },
                                    }
                                  }
                                }
                            }
                            SearchResult::Album(ref album) => {
                                let album_clone = album.clone();
                                let album_for_dl = album.clone();
                                let album_for_override = album.clone();
                                let album_id = album.id.clone();
                                let dl_state = download_states.read().get(&album_id).cloned().unwrap_or_default();
                                let has_folders = !folders.read().is_empty();
                                let effective_state = if !has_folders { DownloadRowState::Disabled } else { dl_state };
                                let current_folders = folders.read().clone();
                                let current_folder_id = selected_folder_id();

                                let is_expanded = expanded_albums.read().contains(&album_id);
                                let cached = album_cache.read().get(&album_id).cloned();
                                let tracks_for_panel: Option<Vec<Track>> = if is_expanded {
                                    cached.map(|awt| awt.tracks)
                                } else {
                                    Some(vec![])
                                };

                                rsx! {
                                  li { key: "{album.id}",
                                    AlbumResult {
                                      is_expanded,
                                      on_toggle: {
                                          let id = album_clone.id.clone();
                                          move || {
                                              toggle_expand(id.clone(), provider);
                                          }
                                      },
                                      on_search_sources: {
                                          let album_for_search = album.clone();
                                          move || {
                                              let query = DownloadQuery::new(vec![]).album(shared::metadata::Album {
                                                  id: album_for_search.id.clone(),
                                                  title: album_for_search.title.clone(),
                                                  artist: album_for_search.artist.clone(),
                                                  release_date: album_for_search.release_date.clone(),
                                                  mbid: album_for_search.mbid.clone(),
                                                  cover_url: album_for_search.cover_url.clone(),
                                                  country: album_for_search.country.clone(),
                                                  media: album_for_search.media.clone(),
                                                  track_count: album_for_search.track_count,
                                              });
                                              spawn(download(query));
                                          }
                                      },
                                      album: album.clone(),
                                      download_state: effective_state,
                                      folders: current_folders,
                                      selected_folder_id: current_folder_id.clone(),
                                      active_menu,
                                      on_download: move |_| {
                                          let query = DownloadQuery::new(vec![]).album(shared::metadata::Album {
                                              id: album_for_dl.id.clone(),
                                              title: album_for_dl.title.clone(),
                                              artist: album_for_dl.artist.clone(),
                                              release_date: album_for_dl.release_date.clone(),
                                              mbid: album_for_dl.mbid.clone(),
                                              cover_url: album_for_dl.cover_url.clone(),
                                              country: album_for_dl.country.clone(),
                                              media: album_for_dl.media.clone(),
                                              track_count: album_for_dl.track_count,
                                          });
                                          handle_auto_download(album_for_dl.id.clone(), query);
                                      },
                                      on_override_download: move |folder: Folder| {
                                          let query = DownloadQuery::new(vec![]).album(shared::metadata::Album {
                                              id: album_for_override.id.clone(),
                                              title: album_for_override.title.clone(),
                                              artist: album_for_override.artist.clone(),
                                              release_date: album_for_override.release_date.clone(),
                                              mbid: album_for_override.mbid.clone(),
                                              cover_url: album_for_override.cover_url.clone(),
                                              country: album_for_override.country.clone(),
                                              media: album_for_override.media.clone(),
                                              track_count: album_for_override.track_count,
                                          });
                                          handle_override_download(album_for_override.id.clone(), query, folder);
                                      },
                                      tracks: tracks_for_panel,
                                      download_states,
                                      on_track_download: move |track: Track| {
                                          handle_track_download(track);
                                      },
                                      on_track_override_download: move |(track, folder): (Track, Folder)| {
                                          handle_track_override_download((track, folder));
                                      },
                                    }
                                  }
                                }
                            }
                            SearchResult::AlbumGroup(ref group) => {
                                let selected = selected_edition
                                    .read()
                                    .get(&group.id)
                                    .copied()
                                    .unwrap_or(0)
                                    .min(group.editions.len().saturating_sub(1));
                                let fallback_album = Album {
                                    id: group.id.clone(),
                                    title: group.title.clone(),
                                    artist: group.artist.clone(),
                                    release_date: group.release_date.clone(),
                                    mbid: group.mbid.clone(),
                                    cover_url: group.cover_url.clone(),
                                    country: None,
                                    media: None,
                                    track_count: None,
                                };
                                let edition = group
                                    .editions
                                    .get(selected)
                                    .cloned()
                                    .unwrap_or(fallback_album);
                                let edition_id = edition.id.clone();
                                let edition_for_search = edition.clone();
                                let edition_for_dl = edition.clone();
                                let edition_for_override = edition;
                                let group_id = group.id.clone();
                                let is_expanded = expanded_albums.read().contains(&edition_id);
                                let cached = album_cache.read().get(&edition_id).cloned();
                                let tracks_for_panel: Option<Vec<Track>> = if is_expanded {
                                    cached.map(|awt| awt.tracks)
                                } else {
                                    Some(vec![])
                                };
                                let dl_state = download_states
                                    .read()
                                    .get(&edition_id)
                                    .cloned()
                                    .unwrap_or_default();
                                let has_folders = !folders.read().is_empty();
                                let effective_state = if !has_folders {
                                    DownloadRowState::Disabled
                                } else {
                                    dl_state
                                };
                                let current_folders = folders.read().clone();
                                let current_folder_id = selected_folder_id();

                                rsx! {
                                  li { key: "{group.id}",
                                    AlbumGroupResult {
                                      editions: group.editions.clone(),
                                      selected,
                                      on_select: move |index: usize| {
                                          selected_edition.write().insert(group_id.clone(), index);
                                      },
                                      is_expanded,
                                      on_toggle: move || {
                                          toggle_expand(edition_id.clone(), provider);
                                      },
                                      on_search_sources: move || {
                                          spawn(download(DownloadQuery::new(vec![]).album(edition_for_search.clone())));
                                      },
                                      download_state: effective_state,
                                      folders: current_folders,
                                      selected_folder_id: current_folder_id.clone(),
                                      active_menu,
                                      tracks: tracks_for_panel,
                                      download_states,
                                      on_download: move |_| {
                                          handle_auto_download(
                                              edition_for_dl.id.clone(),
                                              DownloadQuery::new(vec![]).album(edition_for_dl.clone()),
                                          );
                                      },
                                      on_override_download: move |folder: Folder| {
                                          handle_override_download(
                                              edition_for_override.id.clone(),
                                              DownloadQuery::new(vec![]).album(edition_for_override.clone()),
                                              folder,
                                          );
                                      },
                                      on_track_download: move |track: Track| {
                                          handle_track_download(track);
                                      },
                                      on_track_override_download: move |(track, folder): (Track, Folder)| {
                                          handle_track_override_download((track, folder));
                                      },
                                    }
                                  }
                                }
                            }
                        }
                      }
                    }
                  }
                }
              },
              Some(_) => rsx! {
                div { class: "text-center text-gray-500 py-10 font-mono", "No signals found in the ether." }
              },
              None => rsx! {},
          }
        }

        // Fallback toasts (D-14 through D-17)
        if !fallback_toasts.read().is_empty() {
          div { class: "fixed bottom-4 right-4 flex flex-col-reverse gap-2 z-40 w-80 md:w-96",
            for toast_data in fallback_toasts.read().iter() {
              FallbackToast {
                key: "{toast_data.id}",
                toast: toast_data.clone(),
                on_pick_source: move |results: Vec<DownloadableGroup>| {
                    download_options.set(Some(results));
                },
                on_dismiss: move |id: String| {
                    fallback_toasts.write().retain(|t| t.id != id);
                },
              }
            }
          }
        }
      }
    }
}

fn default_search_filters(options: &SearchFilterOptions) -> SearchFilters {
    let pick = |available: &[String], wanted: &[&str]| -> Vec<String> {
        available
            .iter()
            .filter(|value| wanted.iter().any(|want| value.eq_ignore_ascii_case(want)))
            .cloned()
            .collect()
    };

    SearchFilters {
        release_types: pick(&options.release_types, &["Album", "Single"]),
        secondary_types: pick(&options.secondary_types, &["Soundtrack"]),
        media_formats: Vec::new(),
    }
}
