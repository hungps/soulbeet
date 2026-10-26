use dioxus::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::HtmlAudioElement;

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerTrack {
    pub url: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<u32>,
}

#[derive(Clone, Copy)]
pub struct Player {
    track: Signal<Option<PlayerTrack>>,
    playing: Signal<bool>,
    time: Signal<f64>,
    duration: Signal<f64>,
    error: Signal<Option<String>>,
    audio: Signal<Option<HtmlAudioElement>>,
}

impl Player {
    pub fn current(&self) -> Option<PlayerTrack> {
        self.track.read().clone()
    }

    pub fn is_playing(&self) -> bool {
        *self.playing.read()
    }

    pub fn time(&self) -> f64 {
        *self.time.read()
    }

    pub fn duration(&self) -> f64 {
        *self.duration.read()
    }

    pub fn error(&self) -> Option<String> {
        self.error.read().clone()
    }

    pub fn play_track(&self, track: PlayerTrack) {
        let mut error = self.error;
        let Some(audio) = self.ensure_audio() else {
            error.set(Some("This browser cannot play audio".to_string()));
            return;
        };
        let mut time = self.time;
        let mut duration = self.duration;
        let mut current = self.track;
        error.set(None);
        time.set(0.0);
        duration.set(track.duration.map(f64::from).unwrap_or(0.0));
        current.set(Some(track.clone()));
        audio.set_src(&track.url);
        audio.load();
        if audio.play().is_err() {
            error.set(Some("Could not play this file".to_string()));
        }
    }

    pub fn toggle(&self) {
        let mut error = self.error;
        let Some(audio) = self.audio.peek().clone() else {
            return;
        };
        if audio.paused() {
            error.set(None);
            if audio.play().is_err() {
                error.set(Some("Could not play this file".to_string()));
            }
        } else {
            let _ = audio.pause();
        }
    }

    pub fn stop(&self) {
        let mut current = self.track;
        let mut playing = self.playing;
        let mut time = self.time;
        let mut duration = self.duration;
        let mut error = self.error;
        let audio = self.audio.peek().clone();
        if let Some(audio) = audio {
            let _ = audio.pause();
            audio.set_src("");
            audio.load();
        }
        current.set(None);
        playing.set(false);
        time.set(0.0);
        duration.set(0.0);
        error.set(None);
    }

    pub fn seek(&self, seconds: f64) {
        let mut time = self.time;
        let Some(audio) = self.audio.peek().clone() else {
            return;
        };
        let seconds = seconds.max(0.0);
        audio.set_current_time(seconds);
        time.set(seconds);
    }

    fn ensure_audio(&self) -> Option<HtmlAudioElement> {
        if let Some(audio) = self.audio.peek().clone() {
            return Some(audio);
        }

        let audio = HtmlAudioElement::new().ok()?;
        audio.set_preload("metadata");
        audio.set_volume(1.0);
        if let Some(body) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.body())
        {
            let _ = body.append_child(&audio);
        }
        self.listen(&audio);
        let mut current = self.audio;
        current.set(Some(audio.clone()));
        Some(audio)
    }

    fn listen(&self, audio: &HtmlAudioElement) {
        let target = audio.clone();
        let mut time = self.time;
        let mut duration = self.duration;
        let update_time = Closure::<dyn FnMut()>::new(move || {
            let current = target.current_time();
            if current.is_finite() {
                time.set(current);
            }
            let total = target.duration();
            if total.is_finite() && total > 0.0 {
                duration.set(total);
            }
        });
        let _ = audio
            .add_event_listener_with_callback("timeupdate", update_time.as_ref().unchecked_ref());
        update_time.forget();

        let target = audio.clone();
        let mut duration = self.duration;
        let update_duration = Closure::<dyn FnMut()>::new(move || {
            let total = target.duration();
            if total.is_finite() && total > 0.0 {
                duration.set(total);
            }
        });
        let _ = audio.add_event_listener_with_callback(
            "loadedmetadata",
            update_duration.as_ref().unchecked_ref(),
        );
        update_duration.forget();

        let mut playing = self.playing;
        let on_play = Closure::<dyn FnMut()>::new(move || playing.set(true));
        let _ = audio.add_event_listener_with_callback("play", on_play.as_ref().unchecked_ref());
        on_play.forget();

        let mut playing = self.playing;
        let on_pause = Closure::<dyn FnMut()>::new(move || playing.set(false));
        let _ = audio.add_event_listener_with_callback("pause", on_pause.as_ref().unchecked_ref());
        on_pause.forget();

        let mut playing = self.playing;
        let mut time = self.time;
        let mut duration = self.duration;
        let target = audio.clone();
        let on_ended = Closure::<dyn FnMut()>::new(move || {
            playing.set(false);
            let total = target.duration();
            if total.is_finite() && total > 0.0 {
                time.set(total);
                duration.set(total);
            }
        });
        let _ = audio.add_event_listener_with_callback("ended", on_ended.as_ref().unchecked_ref());
        on_ended.forget();

        let mut error = self.error;
        let mut playing = self.playing;
        let track = self.track;
        let on_error = Closure::<dyn FnMut()>::new(move || {
            if track.peek().is_some() {
                playing.set(false);
                error.set(Some("Could not play this file".to_string()));
            }
        });
        let _ = audio.add_event_listener_with_callback("error", on_error.as_ref().unchecked_ref());
        on_error.forget();
    }

    fn dispose(&self) {
        let audio = self.audio.peek().clone();
        if let Some(audio) = audio {
            let _ = audio.pause();
            audio.set_src("");
            audio.load();
            if let Some(parent) = audio.parent_node() {
                let _ = parent.remove_child(&audio);
            }
        }
    }
}

#[component]
pub fn PlayerProvider(children: Element) -> Element {
    let player = Player {
        track: use_signal(|| None),
        playing: use_signal(|| false),
        time: use_signal(|| 0.0),
        duration: use_signal(|| 0.0),
        error: use_signal(|| None),
        audio: use_signal(|| None),
    };
    use_context_provider(|| player);
    use_drop(move || player.dispose());

    rsx! {
        {children}
    }
}

pub fn use_player() -> Player {
    use_context()
}

pub fn request_audio_url(request_id: &str, item: &str) -> String {
    format!(
        "/api/audio/request?request={}&item={}",
        url_encode(request_id),
        url_encode(item)
    )
}

pub fn library_audio_url(track_id: &str) -> String {
    format!("/api/audio/library/{}", url_encode(track_id))
}

fn url_encode(value: &str) -> String {
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

fn format_seconds(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        return "0:00".to_string();
    }
    let seconds = value as u32;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[component]
pub fn MiniPlayer() -> Element {
    let player = use_player();
    let Some(track) = player.current() else {
        return rsx! {};
    };
    let current = player.time();
    let duration = player.duration().max(0.0);
    let playing = player.is_playing();
    let error = player.error();
    let subtitle = [track.artist.clone(), track.album.clone()]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ");

    rsx! {
        div { class: "px-4 sm:px-6 lg:px-8 pb-3",
            div { class: "bg-beet-panel/95 border border-white/10 rounded-lg px-3 py-2 shadow-2xl",
                div { class: "flex items-center gap-3 min-w-0",
                    div { class: "w-8 h-8 shrink-0 rounded-full border border-beet-leaf/40 bg-beet-leaf/10 flex items-center justify-center text-beet-leaf",
                        svg {
                            class: "w-4 h-4",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "1.8",
                            view_box: "0 0 24 24",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                d: "M9 18V5l12-2v13M9 18a3 3 0 11-6 0 3 3 0 016 0zM21 16a3 3 0 11-6 0 3 3 0 016 0z",
                            }
                        }
                    }
                    div { class: "min-w-0 flex-1",
                        p { class: "text-xs font-bold text-white break-words", "{track.title}" }
                        if !subtitle.is_empty() {
                            p { class: "text-[10px] font-mono text-gray-500 break-words", "{subtitle}" }
                        }
                    }
                    div { class: "flex items-center gap-1 shrink-0",
                        button {
                            class: "p-1.5 rounded-full border border-white/10 text-beet-leaf hover:bg-beet-leaf/10 transition-colors cursor-pointer",
                            title: if playing { "Pause" } else { "Play" },
                            aria_label: if playing { "Pause" } else { "Play" },
                            onclick: move |_| player.toggle(),
                            if playing {
                                svg {
                                    class: "w-3.5 h-3.5",
                                    fill: "currentColor",
                                    view_box: "0 0 24 24",
                                    path { d: "M7 5h4v14H7zM13 5h4v14h-4z" }
                                }
                            } else {
                                svg {
                                    class: "w-3.5 h-3.5",
                                    fill: "currentColor",
                                    view_box: "0 0 24 24",
                                    path { d: "M8 5v14l11-7z" }
                                }
                            }
                        }
                        button {
                            class: "p-1.5 rounded-full border border-transparent text-gray-500 hover:text-white hover:border-white/10 hover:bg-white/5 transition-colors cursor-pointer",
                            title: "Stop",
                            aria_label: "Stop",
                            onclick: move |_| player.stop(),
                            svg {
                                class: "w-3.5 h-3.5",
                                fill: "currentColor",
                                view_box: "0 0 24 24",
                                path { d: "M6 6h12v12H6z" }
                            }
                        }
                    }
                }
                div { class: "flex items-center gap-2 mt-1.5 font-mono text-[10px] text-gray-500",
                    span { class: "w-8 text-right shrink-0", "{format_seconds(current)}" }
                    input {
                        r#type: "range",
                        min: "0",
                        max: "{duration}",
                        step: "0.1",
                        value: "{current}",
                        class: "flex-1 min-w-0 h-1 accent-beet-accent cursor-pointer",
                        oninput: move |event| {
                            if let Ok(value) = event.value().parse::<f64>() {
                                player.seek(value);
                            }
                        },
                    }
                    span { class: "w-8 shrink-0", "{format_seconds(duration)}" }
                }
                if let Some(error) = error {
                    p { class: "text-[10px] font-mono text-red-400 mt-1 break-words", "{error}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_minutes_and_seconds() {
        assert_eq!(format_seconds(-1.0), "0:00");
        assert_eq!(format_seconds(0.0), "0:00");
        assert_eq!(format_seconds(9.9), "0:09");
        assert_eq!(format_seconds(176.0), "2:56");
        assert_eq!(format_seconds(f64::NAN), "0:00");
    }

    #[test]
    fn audio_urls_encode_query_values() {
        assert_eq!(
            request_audio_url("abc", r"music\Artist\01 song.flac"),
            "/api/audio/request?request=abc&item=music%5CArtist%5C01%20song.flac"
        );
    }
}
