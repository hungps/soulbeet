use dioxus::prelude::*;
use shared::metadata::MusicBrainzServer;

use super::connection_status_row::{beets_row, ConnectionStatusRow, ImporterDetails};
use crate::friendly_error;
use crate::settings_context::use_settings;
use crate::use_system_health;

#[component]
pub fn AppConfigManager() -> Element {
    let mut settings = use_settings();
    let health = use_system_health();
    let mut config_resource = use_resource(|| async { api::get_app_config().await });

    let config = match &*config_resource.read() {
        None => {
            return rsx! {
                div { class: "bg-beet-panel border border-white/10 p-6 rounded-lg shadow-2xl relative z-10",
                    div { class: "animate-pulse text-gray-400 font-mono", "Loading..." }
                }
            };
        }
        Some(Err(e)) => {
            let msg = friendly_error(e);
            return rsx! {
                div { class: "bg-beet-panel border border-white/10 p-6 rounded-lg shadow-2xl relative z-10",
                    h2 { class: "text-xl font-bold mb-4 text-beet-accent font-display", "Connections" }
                    div { class: "text-red-400 text-sm font-mono mb-3", "{msg}" }
                    button {
                        class: "text-xs font-mono text-gray-400 hover:text-white underline decoration-dotted cursor-pointer",
                        onclick: move |_| config_resource.restart(),
                        "Retry"
                    }
                }
            };
        }
        Some(Ok(data)) => data.clone(),
    };

    let mut slskd_url = use_signal(|| config.slskd_url.unwrap_or_default());
    let mut slskd_api_key = use_signal(|| config.slskd_api_key.unwrap_or_default());
    let mut slskd_download_path = use_signal(|| config.slskd_download_path.unwrap_or_default());
    let mut download_path = use_signal(|| {
        config
            .download_path
            .unwrap_or_else(|| "/downloads".to_string())
    });
    let mut album_mode = use_signal(|| config.beets_album_mode.unwrap_or(false));
    let mut servers = use_signal(|| config.musicbrainz_servers.unwrap_or_default());
    let mut error = use_signal(String::new);
    let mut success_msg = use_signal(String::new);
    let mut saving = use_signal(|| false);

    let handle_save = move |_| async move {
        error.set(String::new());
        success_msg.set(String::new());
        saving.set(true);

        let config = api::AppConfigValues {
            slskd_url: Some(slskd_url()),
            slskd_api_key: Some(slskd_api_key()),
            slskd_download_path: Some(slskd_download_path()),
            download_path: Some(download_path()),
            beets_album_mode: Some(album_mode()),
            musicbrainz_servers: Some(servers()),
        };

        match api::update_app_config(config).await {
            Ok(saved) => {
                if let Some(rows) = saved.musicbrainz_servers {
                    servers.set(rows);
                }
                let _ = settings.refresh_providers().await;
                success_msg.set("Configuration saved".to_string());
            }
            Err(e) => error.set(friendly_error(&e)),
        }
        saving.set(false);
    };

    let server_rows = servers.read().clone();

    rsx! {
        div { class: "bg-beet-panel border border-white/10 p-6 rounded-lg shadow-2xl relative z-10",
            h2 { class: "text-xl font-bold mb-4 text-beet-accent font-display", "Connections" }

            if !error().is_empty() {
                div { class: "mb-4 p-4 bg-red-900/20 border border-red-500/50 rounded text-red-400 font-mono text-sm",
                    "{error}"
                }
            }
            if !success_msg().is_empty() {
                div { class: "mb-4 p-4 bg-green-900/20 border border-green-500/50 rounded text-green-400 font-mono text-sm",
                    "{success_msg}"
                }
            }

            div { class: "space-y-8 mb-6",
                // Soulseek
                div {
                    h3 { class: "text-sm font-semibold text-white mb-3", "Soulseek (slskd)" }
                    div { class: "space-y-4",
                        div {
                            label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider", "slskd URL" }
                            input {
                                class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                                value: "{slskd_url}",
                                oninput: move |e| slskd_url.set(e.value()),
                                placeholder: "http://localhost:5030",
                            }
                        }
                        div {
                            label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider", "slskd API Key" }
                            input {
                                class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                                value: "{slskd_api_key}",
                                oninput: move |e| slskd_api_key.set(e.value()),
                                placeholder: "Enter slskd API key",
                                "type": "password",
                            }
                        }
                        div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                            div {
                                label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider", "slskd downloads path" }
                                input {
                                    class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                                    aria_label: "slskd downloads path",
                                    value: "{slskd_download_path}",
                                    oninput: move |e| slskd_download_path.set(e.value()),
                                    placeholder: "/downloads (inside slskd)",
                                }
                            }
                            div {
                                label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider", "Soulbeet downloads path" }
                                input {
                                    class: "w-full p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                                    aria_label: "Soulbeet downloads path",
                                    value: "{download_path}",
                                    oninput: move |e| download_path.set(e.value()),
                                    placeholder: "/downloads",
                                }
                            }
                        }
                        p { class: "text-xs text-gray-500 font-mono",
                            "Both paths are optional. They translate a path slskd reports to the path Soulbeet has mounted locally."
                        }
                    }
                }

                // Beets importer
                {
                    let importer = health.importer();
                    let row = beets_row(&importer);
                    let has_details = !importer.issues.is_empty() || !importer.facts.is_empty();
                    rsx! {
                        ConnectionStatusRow {
                            label: "Beets",
                            dot_class: row.dot_class,
                            summary: row.summary,
                            note: row.note,
                            details: has_details.then(|| rsx! { ImporterDetails { health: importer.clone() } }),
                        }
                    }
                }

                // Import behaviour
                div {
                    h3 { class: "text-sm font-semibold text-white mb-3", "Import" }
                    label { class: "flex items-start gap-3 cursor-pointer",
                        input {
                            r#type: "checkbox",
                            class: "mt-1 accent-beet-accent",
                            checked: album_mode(),
                            onchange: move |e| album_mode.set(e.checked()),
                        }
                        div {
                            span { class: "block text-sm text-white", "Album mode" }
                            span { class: "block text-xs text-gray-500 font-mono",
                                "Group downloaded tracks by folder and import them as one album."
                            }
                        }
                    }
                }

                // MusicBrainz servers
                div {
                    div { class: "flex items-center justify-between mb-3",
                        h3 { class: "text-sm font-semibold text-white", "Alternative MusicBrainz servers" }
                        button {
                            class: "text-xs font-mono px-2 py-1 rounded border border-white/10 text-gray-300 hover:text-white hover:bg-white/5 transition-colors cursor-pointer",
                            onclick: move |_| {
                                servers.write().push(MusicBrainzServer {
                                    id: String::new(),
                                    name: String::new(),
                                    host: String::new(),
                                });
                            },
                            "+ Add server"
                        }
                    }
                    p { class: "text-xs text-gray-500 font-mono mb-3",
                        "Each server appears as a selectable metadata provider on the search page. Use host[:port] without a scheme."
                    }

                    if server_rows.is_empty() {
                        div { class: "text-xs text-gray-500 font-mono py-2",
                            "No alternative servers configured."
                        }
                    }

                    div { class: "space-y-3",
                        for (index, server) in server_rows.iter().enumerate() {
                            div { key: "{index}", class: "flex flex-col md:flex-row gap-2 items-stretch",
                                input {
                                    class: "md:w-1/3 p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                                    value: "{server.name}",
                                    placeholder: "Name (e.g. BrainzMash)",
                                    oninput: move |e| {
                                        if let Some(row) = servers.write().get_mut(index) {
                                            row.name = e.value();
                                        }
                                    },
                                }
                                input {
                                    class: "flex-grow p-2 rounded bg-beet-dark border border-white/10 focus:border-beet-accent focus:outline-none text-white font-mono",
                                    value: "{server.host}",
                                    placeholder: "musicbrainz.example.com:8080",
                                    oninput: move |e| {
                                        if let Some(row) = servers.write().get_mut(index) {
                                            row.host = e.value();
                                        }
                                    },
                                }
                                button {
                                    class: "px-3 py-2 rounded border border-white/10 text-gray-500 hover:text-red-400 hover:bg-red-500/10 transition-colors cursor-pointer",
                                    title: "Remove server",
                                    aria_label: "Remove server",
                                    onclick: move |_| {
                                        let mut rows = servers.write();
                                        if index < rows.len() {
                                            rows.remove(index);
                                        }
                                    },
                                    "×"
                                }
                            }
                        }
                    }
                }
            }

            button {
                class: "retro-btn rounded",
                disabled: saving(),
                onclick: handle_save,
                if saving() { "Saving..." } else { "Save Configuration" }
            }
        }
    }
}
