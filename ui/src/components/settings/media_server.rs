use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use shared::media_server::{
    MediaPathMapping, MediaServerKind, MediaServerView, PlexServerOption,
};
use shared::system::MediaServerStatus;

use api::PlexLinkPoll;

use crate::use_auth;

#[cfg(target_arch = "wasm32")]
fn open_plex_window() -> Option<web_sys::Window> {
    web_sys::window()?
        .open_with_url_and_target("about:blank", "_blank")
        .ok()
        .flatten()
}

#[cfg(not(target_arch = "wasm32"))]
fn open_plex_window() -> Option<()> {
    None
}

fn server_url(server: &PlexServerOption, secure: bool) -> String {
    server.url(secure)
}

fn status_label(status: MediaServerStatus) -> (&'static str, &'static str) {
    match status {
        MediaServerStatus::Connected => ("bg-green-400", "Connected"),
        MediaServerStatus::MissingReportRealPath => ("bg-yellow-500", "ReportRealPath disabled"),
        MediaServerStatus::InvalidCredentials => ("bg-red-500", "Invalid credentials"),
        MediaServerStatus::Offline => ("bg-gray-500", "Unreachable"),
        MediaServerStatus::Unknown => ("bg-gray-500", "Not configured"),
    }
}

fn server_message(error: &ServerFnError) -> String {
    match error {
        ServerFnError::ServerError { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

#[component]
pub fn MediaServerManager() -> Element {
    let auth = use_auth();
    let mut resource = use_resource(|| async { api::get_media_server_settings().await });
    let mut selected_kind = use_signal(MediaServerKind::default);
    let mut initialized = use_signal(|| false);

    use_effect(move || {
        if let Some(Ok(settings)) = &*resource.read() {
            if !initialized() {
                selected_kind.set(settings.active);
                initialized.set(true);
            }
        }
    });

    let on_changed = use_callback(move |_| {
        let mut resource = resource;
        resource.restart();
        let mut auth = auth;
        spawn(async move {
            auth.refresh().await;
        });
    });

    let settings = match &*resource.read() {
        None => {
            return rsx! {
                div { class: "bg-beet-panel border border-white/10 p-6 rounded-lg shadow-2xl relative z-10",
                    div { class: "animate-pulse text-gray-400 font-mono", "Loading..." }
                }
            };
        }
        Some(Err(e)) => {
            return rsx! {
                div { class: "bg-beet-panel border border-white/10 p-6 rounded-lg shadow-2xl relative z-10",
                    h2 { class: "text-xl font-bold mb-4 text-beet-accent font-display", "Media Server" }
                    div { class: "text-red-400 text-sm font-mono mb-3", "{server_message(e)}" }
                    button {
                        class: "text-xs font-mono text-gray-400 hover:text-white underline decoration-dotted cursor-pointer",
                        onclick: move |_| resource.restart(),
                        "Retry"
                    }
                }
            };
        }
        Some(Ok(settings)) => settings.clone(),
    };

    let kind = selected_kind();
    let mappings: Vec<MediaPathMapping> = settings
        .mappings
        .iter()
        .filter(|m| m.kind == kind)
        .cloned()
        .collect();

    rsx! {
        div { class: "space-y-6",
            div { class: "bg-beet-panel border border-white/10 p-6 rounded-lg shadow-2xl relative z-10",
                h2 { class: "text-xl font-bold mb-4 text-beet-accent font-display", "Media Server" }

                div { class: "mb-6",
                    label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider",
                        "Server"
                    }
                    select {
                        class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                        value: "{kind}",
                        onchange: move |e| selected_kind.set(MediaServerKind::from(e.value().as_str())),
                        option { value: "navidrome", "Navidrome" }
                        option { value: "plex", "Plex" }
                    }
                    p { class: "text-xs text-gray-500 mt-1 font-mono",
                        "Choose which media server to configure. Connecting or saving makes it the active server."
                    }
                }

                match kind {
                    MediaServerKind::Navidrome => rsx! {
                        NavidromePanel {
                            config: settings.navidrome.clone(),
                            active: settings.active == MediaServerKind::Navidrome,
                            on_changed,
                        }
                    },
                    MediaServerKind::Plex => {
                        let plex_key = format!(
                            "{}|{}|{}|{}|{}",
                            settings.plex.as_ref().map(|c| c.linked).unwrap_or(false),
                            settings
                                .plex
                                .as_ref()
                                .and_then(|c| c.url.clone())
                                .unwrap_or_default(),
                            settings.plex.as_ref().map(|c| c.secure).unwrap_or(false),
                            settings
                                .plex
                                .as_ref()
                                .and_then(|c| c.server_id.clone())
                                .unwrap_or_default(),
                            settings.plex_servers.len(),
                        );
                        rsx! {
                            PlexPanel {
                                key: "{plex_key}",
                                config: settings.plex.clone(),
                                servers: settings.plex_servers.clone(),
                                active: settings.active == MediaServerKind::Plex,
                                on_changed,
                            }
                        }
                    }
                }
            }

            div { class: "bg-beet-panel border border-white/10 p-6 rounded-lg shadow-2xl relative z-10",
                h3 { class: "text-lg font-bold mb-1 text-beet-accent font-display", "Path Mapping" }
                p { class: "text-xs text-gray-500 mb-4 font-mono",
                    "Map the path Soulbeet sees to the path the media server sees, for example /medias/music → /volume1/medias/music."
                }
                PathMappings { kind, mappings, on_changed }
            }
        }
    }
}

#[component]
fn NavidromePanel(
    config: Option<MediaServerView>,
    active: bool,
    on_changed: EventHandler<()>,
) -> Element {
    let mut url = use_signal(String::new);
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut last = use_signal(|| None::<MediaServerView>);
    let mut error = use_signal(String::new);
    let mut success = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let effect_config = config.clone();
    use_effect(move || {
        let next = effect_config.clone();
        if last() != next {
            url.set(
                next.as_ref()
                    .and_then(|c| c.url.clone())
                    .unwrap_or_default(),
            );
            username.set(
                next.as_ref()
                    .and_then(|c| c.account_name.clone())
                    .unwrap_or_default(),
            );
            password.set(String::new());
            last.set(next);
        }
    });

    let linked = config.as_ref().map(|c| c.linked).unwrap_or(false);
    let status = config
        .as_ref()
        .map(|c| c.status)
        .unwrap_or(MediaServerStatus::Unknown);
    let (dot, label) = status_label(status);

    let connect = move |_| {
        if busy() {
            return;
        }
        error.set(String::new());
        success.set(String::new());
        busy.set(true);
        let url_value = url();
        let user_value = username();
        let password_value = if password().is_empty() {
            None
        } else {
            Some(password())
        };
        spawn(async move {
            match api::connect_navidrome(url_value, user_value, password_value).await {
                Ok(()) => {
                    success.set("Connected to Navidrome".to_string());
                    password.set(String::new());
                    on_changed.call(());
                }
                Err(e) => error.set(server_message(&e)),
            }
            busy.set(false);
        });
    };

    let disconnect = move |_| {
        if busy() {
            return;
        }
        error.set(String::new());
        success.set(String::new());
        busy.set(true);
        spawn(async move {
            match api::disconnect_navidrome().await {
                Ok(()) => {
                    success.set("Disconnected from Navidrome".to_string());
                    on_changed.call(());
                }
                Err(e) => error.set(server_message(&e)),
            }
            busy.set(false);
        });
    };

    let make_active = move |_| {
        if busy() {
            return;
        }
        error.set(String::new());
        success.set(String::new());
        busy.set(true);
        spawn(async move {
            match api::set_active_media_server(MediaServerKind::Navidrome).await {
                Ok(()) => {
                    success.set("Navidrome is now active".to_string());
                    on_changed.call(());
                }
                Err(e) => error.set(server_message(&e)),
            }
            busy.set(false);
        });
    };

    rsx! {
        div { class: "space-y-4",
            div { class: "flex items-center gap-2",
                span { class: format!("w-2 h-2 rounded-full {dot}") }
                span { class: "text-xs font-mono text-gray-400", "{label}" }
                if active {
                    span { class: "text-[10px] font-mono uppercase tracking-wider text-beet-accent", "Active" }
                }
            }

            if !error().is_empty() {
                div { class: "p-3 bg-red-900/20 border border-red-500/50 rounded text-red-400 font-mono text-sm",
                    "{error}"
                }
            }
            if !success().is_empty() {
                div { class: "p-3 bg-green-900/20 border border-green-500/50 rounded text-green-400 font-mono text-sm",
                    "{success}"
                }
            }

            div {
                label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider", "Navidrome URL" }
                input {
                    class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                    value: "{url}",
                    oninput: move |e| url.set(e.value()),
                    placeholder: "http://localhost:4533",
                }
            }
            div {
                label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider", "Username" }
                input {
                    class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                    value: "{username}",
                    oninput: move |e| username.set(e.value()),
                    placeholder: "Navidrome username",
                }
            }
            div {
                label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider", "Password" }
                input {
                    class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                    value: "{password}",
                    oninput: move |e| password.set(e.value()),
                    r#type: "password",
                    placeholder: if linked { "Leave blank to keep the current password" } else { "Navidrome password" },
                }
            }

            div { class: "flex flex-wrap items-center gap-2",
                button {
                    class: "retro-btn rounded",
                    disabled: busy(),
                    onclick: connect,
                    if busy() { "Connecting..." } else { "Connect" }
                }
                if linked && !active {
                    button {
                        class: "px-4 py-2 rounded border border-white/10 text-gray-300 hover:text-white hover:border-beet-accent text-sm font-mono transition-colors",
                        disabled: busy(),
                        onclick: make_active,
                        "Use this server"
                    }
                }
                if linked {
                    button {
                        class: "px-4 py-2 rounded border border-red-500/40 text-red-400 hover:bg-red-500/10 text-sm font-mono transition-colors",
                        disabled: busy(),
                        onclick: disconnect,
                        "Disconnect"
                    }
                }
            }
        }
    }
}

#[component]
fn PlexPanel(
    config: Option<MediaServerView>,
    servers: Vec<PlexServerOption>,
    active: bool,
    on_changed: EventHandler<()>,
) -> Element {
    let mut secure = use_signal(|| config.as_ref().map(|c| c.secure).unwrap_or(false));
    let mut selected = use_signal(|| 0usize);
    let mut last_servers = use_signal(|| None::<Vec<PlexServerOption>>);
    let mut last_config = use_signal(|| None::<MediaServerView>);
    let mut error = use_signal(String::new);
    let mut success = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let secure_config = config.clone();
    use_effect(move || {
        let next = secure_config.clone();
        if last_config() != next {
            if let Some(config) = &next {
                secure.set(config.secure);
            }
            last_config.set(next);
        }
    });

    let effect_config = config.clone();
    let effect_servers = servers.clone();
    use_effect(move || {
        let next = effect_servers.clone();
        if last_servers() != Some(next.clone()) {
            let configured = effect_config.as_ref().and_then(|c| c.url.clone());
            let secure_value = secure();
            let index = next
                .iter()
                .position(|server| configured.as_deref() == Some(server_url(server, secure_value).as_str()))
                .unwrap_or(0);
            selected.set(index);
            last_servers.set(Some(next));
        }
    });

    let linked = config.as_ref().map(|c| c.linked).unwrap_or(false);
    let account_name = config
        .as_ref()
        .and_then(|c| c.account_name.clone())
        .unwrap_or_else(|| "Plex account".to_string());
    let status = config
        .as_ref()
        .map(|c| c.status)
        .unwrap_or(MediaServerStatus::Unknown);
    let (dot, label) = status_label(status);

    let handle_link = move |_| {
        if busy() {
            return;
        }
        error.set(String::new());
        success.set(String::new());
        busy.set(true);
        let popup = open_plex_window();
        spawn(async move {
            let pin = match api::create_plex_pin().await {
                Ok(pin) => pin,
                Err(e) => {
                    error.set(server_message(&e));
                    busy.set(false);
                    return;
                }
            };

            let opened = match &popup {
                #[cfg(target_arch = "wasm32")]
                Some(window) => window.location().set_href(&pin.auth_url).is_ok(),
                #[cfg(not(target_arch = "wasm32"))]
                Some(_) => false,
                None => false,
            };
            if !opened {
                error.set(
                    "Could not open the Plex login window. Allow pop-ups and try again."
                        .to_string(),
                );
                busy.set(false);
                return;
            }

            for _ in 0..150 {
                TimeoutFuture::new(2000).await;
                match api::poll_plex_link(pin.id.clone()).await {
                    Ok(PlexLinkPoll::Pending) => continue,
                    Ok(PlexLinkPoll::Complete { .. }) => {
                        success.set("Plex account linked".to_string());
                        busy.set(false);
                        on_changed.call(());
                        return;
                    }
                    Err(e) => {
                        error.set(server_message(&e));
                        busy.set(false);
                        return;
                    }
                }
            }

            error.set("Plex login timed out. Please try again.".to_string());
            busy.set(false);
        });
    };

    let save_servers = servers.clone();
    let save = move |_| {
        if busy() {
            return;
        }
        let Some(server) = save_servers.get(selected()) else {
            error.set("Select a Plex server first".to_string());
            return;
        };
        error.set(String::new());
        success.set(String::new());
        busy.set(true);
        let url = server_url(server, secure());
        let server_id = server.machine_identifier.clone();
        let server_name = server.name.clone();
        spawn(async move {
            match api::save_plex_config(url, secure(), Some(server_id), Some(server_name)).await {
                Ok(()) => {
                    success.set("Plex server saved".to_string());
                    on_changed.call(());
                }
                Err(e) => error.set(server_message(&e)),
            }
            busy.set(false);
        });
    };

    let auto_detect = move |_| {
        if busy() {
            return;
        }
        error.set(String::new());
        success.set(String::new());
        busy.set(true);
        spawn(async move {
            match api::auto_select_plex_server().await {
                Ok(()) => {
                    success.set("Detected a reachable Plex server".to_string());
                    on_changed.call(());
                }
                Err(e) => error.set(server_message(&e)),
            }
            busy.set(false);
        });
    };

    let disconnect = move |_| {
        if busy() {
            return;
        }
        error.set(String::new());
        success.set(String::new());
        busy.set(true);
        spawn(async move {
            match api::disconnect_plex().await {
                Ok(()) => {
                    success.set("Plex account unlinked".to_string());
                    on_changed.call(());
                }
                Err(e) => error.set(server_message(&e)),
            }
            busy.set(false);
        });
    };

    let make_active = move |_| {
        if busy() {
            return;
        }
        error.set(String::new());
        success.set(String::new());
        busy.set(true);
        spawn(async move {
            match api::set_active_media_server(MediaServerKind::Plex).await {
                Ok(()) => {
                    success.set("Plex is now active".to_string());
                    on_changed.call(());
                }
                Err(e) => error.set(server_message(&e)),
            }
            busy.set(false);
        });
    };

    rsx! {
        div { class: "space-y-4",
            div { class: "flex items-center gap-2",
                span { class: format!("w-2 h-2 rounded-full {dot}") }
                span { class: "text-xs font-mono text-gray-400", "{label}" }
                if linked {
                    span { class: "text-xs font-mono text-gray-300", "{account_name}" }
                }
                if active {
                    span { class: "text-[10px] font-mono uppercase tracking-wider text-beet-accent", "Active" }
                }
            }

            if !error().is_empty() {
                div { class: "p-3 bg-red-900/20 border border-red-500/50 rounded text-red-400 font-mono text-sm",
                    "{error}"
                }
            }
            if !success().is_empty() {
                div { class: "p-3 bg-green-900/20 border border-green-500/50 rounded text-green-400 font-mono text-sm",
                    "{success}"
                }
            }

            if !linked {
                p { class: "text-sm text-gray-400 font-mono",
                    "Sign in with your plex.tv account to link Plex. Soulbeet stores the account token and uses it for this server."
                }
                button {
                    class: "retro-btn rounded",
                    disabled: busy(),
                    onclick: handle_link,
                    if busy() { "Waiting for Plex..." } else { "Login with Plex" }
                }
            } else {
                div {
                    label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider", "Plex server" }
                    select {
                        class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                        value: "{selected}",
                        onchange: {
                            let servers = servers.clone();
                            move |e| {
                                let index = e.value().parse().unwrap_or(0);
                                selected.set(index);
                                if let Some(server) = servers.get(index) {
                                    secure.set(server.secure);
                                }
                            }
                        },
                        for (index, server) in servers.iter().enumerate() {
                            option {
                                value: "{index}",
                                selected: index == selected(),
                                "{server.name} — {server.address}:{server.port} ({connection_label(server)})"
                            }
                        }
                    }
                    if servers.is_empty() {
                        p { class: "text-xs text-amber-400 mt-1 font-mono",
                            "plex.tv returned no servers for this account."
                        }
                    }
                }

                label { class: "flex items-center gap-2 text-sm font-mono text-gray-300 cursor-pointer",
                    input {
                        r#type: "checkbox",
                        checked: secure(),
                        onchange: move |e| secure.set(e.checked()),
                    }
                    "Use secure connection (HTTPS)"
                }

                div { class: "flex flex-wrap items-center gap-2",
                    button {
                        class: "retro-btn rounded",
                        disabled: busy() || servers.is_empty(),
                        onclick: save,
                        if busy() { "Saving..." } else { "Save" }
                    }
                    button {
                        class: "px-4 py-2 rounded border border-white/10 text-gray-300 hover:text-white hover:border-beet-leaf text-sm font-mono transition-colors",
                        disabled: busy(),
                        onclick: auto_detect,
                        "Auto-detect"
                    }
                    if !active {
                        button {
                            class: "px-4 py-2 rounded border border-white/10 text-gray-300 hover:text-white hover:border-beet-accent text-sm font-mono transition-colors",
                            disabled: busy(),
                            onclick: make_active,
                            "Use this server"
                        }
                    }
                    button {
                        class: "px-4 py-2 rounded border border-red-500/40 text-red-400 hover:bg-red-500/10 text-sm font-mono transition-colors",
                        disabled: busy(),
                        onclick: disconnect,
                        "Logout"
                    }
                }
            }
        }
    }
}

fn connection_label(server: &PlexServerOption) -> String {
    let kind = if server.relay {
        "relay"
    } else if server.local {
        "LAN"
    } else {
        "WAN"
    };
    format!("{kind}, {}", if server.secure { "https" } else { "http" })
}

#[component]
fn PathMappings(
    kind: MediaServerKind,
    mappings: Vec<MediaPathMapping>,
    on_changed: EventHandler<()>,
) -> Element {
    let mut local_path = use_signal(String::new);
    let mut remote_path = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let add = move |_| {
        if busy() || local_path().trim().is_empty() || remote_path().trim().is_empty() {
            return;
        }
        error.set(String::new());
        busy.set(true);
        let local = local_path();
        let remote = remote_path();
        let kind = kind;
        spawn(async move {
            match api::add_path_mapping(kind, local, remote).await {
                Ok(_) => {
                    local_path.set(String::new());
                    remote_path.set(String::new());
                    on_changed.call(());
                }
                Err(e) => error.set(server_message(&e)),
            }
            busy.set(false);
        });
    };

    rsx! {
        div { class: "space-y-3",
            if !error().is_empty() {
                div { class: "p-3 bg-red-900/20 border border-red-500/50 rounded text-red-400 font-mono text-sm",
                    "{error}"
                }
            }

            if mappings.is_empty() {
                p { class: "text-xs text-gray-500 font-mono", "No mappings configured." }
            } else {
                div { class: "space-y-2",
                    for mapping in mappings.iter().cloned() {
                        div { key: "{mapping.id}", class: "flex items-center gap-2 bg-beet-dark/70 border border-white/10 rounded p-2",
                            div { class: "flex-1 min-w-0 grid grid-cols-1 sm:grid-cols-2 gap-1",
                                span { class: "text-xs font-mono text-gray-300 truncate", "{mapping.local_path}" }
                                span { class: "text-xs font-mono text-gray-500 truncate", "→ {mapping.remote_path}" }
                            }
                            button {
                                class: "text-xs font-mono text-red-400 hover:text-red-300 px-2",
                                disabled: busy(),
                                onclick: {
                                    let id = mapping.id.clone();
                                    move |_| {
                                        if busy() {
                                            return;
                                        }
                                        error.set(String::new());
                                        busy.set(true);
                                        let id = id.clone();
                                        spawn(async move {
                                            match api::delete_path_mapping(id).await {
                                                Ok(()) => on_changed.call(()),
                                                Err(e) => error.set(server_message(&e)),
                                            }
                                            busy.set(false);
                                        });
                                    }
                                },
                                "Remove"
                            }
                        }
                    }
                }
            }

            div { class: "grid grid-cols-1 sm:grid-cols-2 gap-2",
                input {
                    class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono text-sm",
                    value: "{local_path}",
                    oninput: move |e| local_path.set(e.value()),
                    placeholder: "Soulbeet path",
                }
                input {
                    class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono text-sm",
                    value: "{remote_path}",
                    oninput: move |e| remote_path.set(e.value()),
                    placeholder: "Media server path",
                }
            }
            button {
                class: "px-4 py-2 rounded border border-white/10 text-gray-300 hover:text-white hover:border-beet-accent text-sm font-mono transition-colors",
                disabled: busy(),
                onclick: add,
                "Add mapping"
            }
        }
    }
}
