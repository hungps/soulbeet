use std::future::Future;
use std::pin::Pin;

use api::auth::AuthResponse;
use api::{create_plex_pin, poll_plex_login, PlexLoginPoll};
use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use shared::media_server::{AuthServers, MediaServerKind};

use crate::Logo;

type LoginCallback = Callback<
    (String, String, Option<MediaServerKind>),
    Pin<Box<dyn Future<Output = Result<(), String>>>>,
>;

#[derive(Props, PartialEq, Clone)]
pub struct Props {
    login: LoginCallback,
    on_plex_login: EventHandler<AuthResponse>,
    servers: AuthServers,
}

#[cfg(target_arch = "wasm32")]
type PlexWindow = web_sys::Window;
#[cfg(not(target_arch = "wasm32"))]
type PlexWindow = ();

#[cfg(target_arch = "wasm32")]
fn open_plex_window() -> Option<PlexWindow> {
    web_sys::window()?
        .open_with_url_and_target_and_features(
            "about:blank",
            "soulbeet_plex_login",
            "popup=yes,width=560,height=760,menubar=no,toolbar=no,location=no,status=no,resizable=yes,scrollbars=yes",
        )
        .ok()
        .flatten()
}

#[cfg(not(target_arch = "wasm32"))]
fn open_plex_window() -> Option<PlexWindow> {
    None
}

#[cfg(target_arch = "wasm32")]
fn close_plex_window(window: Option<&PlexWindow>) {
    if let Some(window) = window {
        let _ = window.close();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn close_plex_window(_window: Option<&PlexWindow>) {}

fn tab_columns(count: usize) -> &'static str {
    match count {
        3 => "grid-cols-3",
        2 => "grid-cols-2",
        _ => "grid-cols-1",
    }
}

fn login_error(error: &ServerFnError) -> String {
    match error {
        ServerFnError::ServerError { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

#[component]
pub fn Login(props: Props) -> Element {
    let mut username = use_signal(|| "".to_string());
    let mut password = use_signal(|| "".to_string());
    let mut error = use_signal(|| "".to_string());
    let mut tab = use_signal(|| {
        if props.servers.any() {
            Some(props.servers.default_kind())
        } else {
            None
        }
    });
    let mut plex_busy = use_signal(|| false);
    let mut plex_status = use_signal(|| "".to_string());

    let mut tabs: Vec<(Option<MediaServerKind>, &'static str)> = vec![(None, "Local")];
    if props.servers.plex {
        tabs.push((Some(MediaServerKind::Plex), "Plex"));
    }
    if props.servers.navidrome {
        tabs.push((Some(MediaServerKind::Navidrome), "Navidrome"));
    }

    let handle_login = move || {
        let user = username.read().to_string();
        let pass = password.read().to_string();
        let selected = tab();
        spawn(async move {
            error.set("".to_string());
            match props.login.call((user, pass, selected)).await {
                Ok(_) => {
                    // Login success logic usually handled by parent/router redirect
                }
                Err(e) => error.set(e),
            }
        });
    };

    let handle_plex_login = move |_| {
        if plex_busy() {
            return;
        }
        error.set("".to_string());
        plex_status.set("Waiting for Plex...".to_string());
        plex_busy.set(true);

        // Open synchronously so popup blockers do not reject the window.
        let popup = open_plex_window();
        spawn(async move {
            let pin = match create_plex_pin().await {
                Ok(pin) => pin,
                Err(e) => {
                    error.set(login_error(&e));
                    plex_status.set("".to_string());
                    plex_busy.set(false);
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
                close_plex_window(popup.as_ref());
                error.set(
                    "Could not open the Plex login window. Allow pop-ups and try again."
                        .to_string(),
                );
                plex_status.set("".to_string());
                plex_busy.set(false);
                return;
            }

            for _ in 0..150 {
                TimeoutFuture::new(2000).await;
                match poll_plex_login(pin.id.clone()).await {
                    Ok(PlexLoginPoll::Pending) => continue,
                    Ok(PlexLoginPoll::Complete(response)) => {
                        close_plex_window(popup.as_ref());
                        plex_status.set("".to_string());
                        plex_busy.set(false);
                        props.on_plex_login.call(response);
                        return;
                    }
                    Err(e) => {
                        close_plex_window(popup.as_ref());
                        error.set(login_error(&e));
                        plex_status.set("".to_string());
                        plex_busy.set(false);
                        return;
                    }
                }
            }

            close_plex_window(popup.as_ref());
            error.set("Plex login timed out. Please try again.".to_string());
            plex_status.set("".to_string());
            plex_busy.set(false);
        });
    };

    rsx! {
      div { class: "flex flex-col items-center justify-center min-h-screen text-white font-display",
        // bg decorations
        div { class: "fixed top-1/4 -left-10 w-64 h-64 bg-beet-accent/10 rounded-full blur-[150px] pointer-events-none max-md:hidden" }
        div { class: "fixed bottom-1/4 -right-10 w-64 h-64 bg-beet-leaf/10 rounded-full blur-[150px] pointer-events-none max-md:hidden" }

        div { class: "p-8 bg-beet-panel border border-white/10 rounded-lg shadow-2xl w-full max-w-md relative z-10",
          // Header
          div { class: "flex flex-col items-center mb-8",
            Logo {
              size: 64,
              class: "mb-4 drop-shadow-[0_0_14px_rgba(255,0,255,0.5)] hover:-translate-y-1 transition-transform",
            }
            h1 { class: "text-2xl font-bold tracking-tighter uppercase text-transparent bg-clip-text bg-gradient-to-r from-white to-gray-400",
              "Soulbeet"
            }
            p { class: "text-sm text-beet-leaf font-mono mt-2 tracking-widest",
              "YOUR LIBRARY MANAGER"
            }
          }

          if tabs.len() > 1 {
            div {
              class: format!(
                  "grid {} gap-1 bg-beet-dark border border-white/10 rounded p-1 mb-6",
                  tab_columns(tabs.len()),
              ),
              for (kind, label) in tabs.iter().cloned() {
                button {
                  key: "{label}",
                  r#type: "button",
                  class: format!(
                      "py-2 rounded text-xs font-mono uppercase tracking-wider transition-colors {}",
                      if tab() == kind {
                          "bg-beet-accent/20 text-beet-accent"
                      } else {
                          "text-gray-400 hover:text-white"
                      },
                  ),
                  onclick: move |_| tab.set(kind),
                  "{label}"
                }
              }
            }
          }

          if tab() == Some(MediaServerKind::Plex) {
            button {
              r#type: "button",
              class: "w-full retro-btn flex justify-center items-center gap-2 group",
              disabled: plex_busy(),
              onclick: handle_plex_login,
              if plex_busy() {
                "{plex_status}"
              } else {
                span { "LOGIN WITH PLEX" }
                svg {
                  class: "w-4 h-4 group-hover:translate-x-1 transition-transform",
                  fill: "none",
                  view_box: "0 0 24 24",
                  stroke: "currentColor",
                  path {
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    stroke_width: "2",
                    d: "M14 5l7 7m0 0l-7 7m7-7H3",
                  }
                }
              }
            }
          } else {
            form {
              class: "space-y-6",
              onsubmit: move |e| {
                  e.prevent_default();
                  handle_login();
              },
              div {
                label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider",
                  "Username"
                }
                input {
                  class: "w-full bg-beet-dark border border-white/10 rounded p-3 text-white focus:outline-none focus:border-beet-accent focus:shadow-[0_0_10px_rgba(217,70,239,0.3)] transition-all font-mono",
                  value: "{username}",
                  oninput: move |e| username.set(e.value()),
                  "type": "text",
                  placeholder: "Enter username",
                }
              }
              div {
                label { class: "block text-xs font-mono text-gray-400 mb-1 uppercase tracking-wider",
                  "Password"
                }
                input {
                  class: "w-full bg-beet-dark border border-white/10 rounded p-3 text-white focus:outline-none focus:border-beet-accent focus:shadow-[0_0_10px_rgba(217,70,239,0.3)] transition-all font-mono",
                  value: "{password}",
                  oninput: move |e| password.set(e.value()),
                  "type": "password",
                  placeholder: "Enter password",
                }
              }

              button {
                class: "w-full retro-btn flex justify-center items-center gap-2 group",
                "type": "submit",
                span { "AUTHENTICATE" }
                svg {
                  class: "w-4 h-4 group-hover:translate-x-1 transition-transform",
                  fill: "none",
                  view_box: "0 0 24 24",
                  stroke: "currentColor",
                  path {
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    stroke_width: "2",
                    d: "M14 5l7 7m0 0l-7 7m7-7H3",
                  }
                }
              }
            }
          }

          if !error().is_empty() {
            div { class: "mt-6 p-3 bg-red-500/10 border border-red-500/50 rounded text-red-400 text-sm font-mono flex items-center gap-2",
              svg {
                class: "w-4 h-4",
                fill: "none",
                view_box: "0 0 24 24",
                stroke: "currentColor",
                path {
                  stroke_linecap: "round",
                  stroke_linejoin: "round",
                  stroke_width: "2",
                  d: "M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z",
                }
              }
              "{error}"
            }
          }
        }
      }
    }
}
