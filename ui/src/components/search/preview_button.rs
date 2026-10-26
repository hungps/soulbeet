use dioxus::prelude::*;

use crate::{use_auth, use_player, PlayerTrack};

#[derive(Clone, Copy, PartialEq)]
enum State {
    Idle,
    Loading,
    /// Deezer has no clip for this result.
    Missing,
}

/// `M:SS` or `H:MM:SS` as seconds, the way metadata providers write a length.
pub fn length_in_seconds(value: &str) -> Option<u32> {
    let mut seconds = 0u32;
    let mut parts = value.trim().split(':').collect::<Vec<_>>();
    if parts.len() > 3 || parts.iter().any(|part| part.trim().is_empty()) {
        return None;
    }
    for part in parts.drain(..) {
        seconds = seconds * 60 + part.trim().parse::<u32>().ok()?;
    }
    Some(seconds)
}

/// Plays the 30-second preview clip of a search result, so a release can be
/// heard before its sources are searched. The clip is the catalogue's, not the
/// peer's file: a Soulseek source can only be heard once it is downloaded.
#[component]
pub fn PreviewButton(
    artist: String,
    title: String,
    #[props(default)] album: String,
    #[props(default)] duration: Option<u32>,
) -> Element {
    let auth = use_auth();
    let player = use_player();
    let mut state = use_signal(|| State::Idle);
    let loading = state() == State::Loading;
    let missing = state() == State::Missing;
    let hint = match state() {
        State::Missing => "No preview available",
        State::Loading | State::Idle => "Play a 30-second preview",
    };

    rsx! {
        button {
            class: "p-2 rounded-full hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-40 disabled:pointer-events-none shrink-0",
            title: "{hint}",
            aria_label: "{hint}",
            disabled: loading || missing,
            onclick: move |evt: MouseEvent| {
                evt.stop_propagation();
                if loading || missing {
                    return;
                }
                state.set(State::Loading);
                let artist = artist.clone();
                let title = title.clone();
                let album = album.clone();
                spawn(async move {
                    let clip = auth
                        .call(api::preview_clip(api::PreviewQuery {
                            artist: artist.clone(),
                            title: title.clone(),
                            duration,
                        }))
                        .await
                        .ok()
                        .flatten();
                    match clip {
                        Some(url) => {
                            state.set(State::Idle);
                            player.play_track(PlayerTrack {
                                url,
                                title: format!("Preview · {title}"),
                                artist,
                                album,
                                duration: Some(30),
                            });
                        }
                        None => state.set(State::Missing),
                    }
                });
            },
            if loading {
                div { class: "w-4 h-4 animate-spin rounded-full border-2 border-transparent border-t-beet-accent border-b-beet-accent" }
            } else {
                svg {
                    class: "w-4 h-4 text-gray-500 rotate-0",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "2",
                    view_box: "0 0 24 24",
                    path {
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        d: "M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z",
                    }
                    path {
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        d: "M21 12a9 9 0 11-18 0 9 9 0 0118 0z",
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::length_in_seconds;

    #[test]
    fn provider_lengths_read_as_seconds() {
        assert_eq!(length_in_seconds("2:56"), Some(176));
        assert_eq!(length_in_seconds("1:02:03"), Some(3723));
        assert_eq!(length_in_seconds("0:09"), Some(9));
        assert_eq!(length_in_seconds(""), None);
        assert_eq!(length_in_seconds("3:xx"), None);
    }
}
