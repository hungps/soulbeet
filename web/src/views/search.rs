use dioxus::prelude::*;
use ui::{Search, SearchUrlState, SearchUrlSync};

#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn parse_search_state(query: &str) -> SearchUrlState {
    let query = query.strip_prefix('?').unwrap_or(query);
    let mut state = SearchUrlState::default();

    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "q" => state.query = value.into_owned(),
            "artist" => state.artist = Some(value.into_owned()).filter(|value| !value.is_empty()),
            "release" => state.release = Some(value.into_owned()).filter(|value| !value.is_empty()),
            "type" => {
                state.search_type = Some(value.into_owned()).filter(|value| !value.is_empty())
            }
            _ => {}
        }
    }

    state
}

#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn format_search_state(state: &SearchUrlState) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());

    if !state.query.trim().is_empty() {
        serializer.append_pair("q", &state.query);
    }
    if let Some(artist) = state.artist.as_deref().filter(|value| !value.is_empty()) {
        serializer.append_pair("artist", artist);
    }
    if let Some(release) = state.release.as_deref().filter(|value| !value.is_empty()) {
        serializer.append_pair("release", release);
    }
    if let Some(search_type) = state
        .search_type
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        serializer.append_pair("type", search_type);
    }

    serializer.finish()
}

#[cfg(feature = "web")]
fn read_search_state() -> SearchUrlState {
    let Some(window) = web_sys::window() else {
        return SearchUrlState::default();
    };
    let search = window.location().search().unwrap_or_default();

    parse_search_state(&search)
}

#[cfg(not(feature = "web"))]
fn read_search_state() -> SearchUrlState {
    SearchUrlState::default()
}

#[cfg(feature = "web")]
fn write_search_state(state: SearchUrlState) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let path = window
        .location()
        .pathname()
        .unwrap_or_else(|_| "/".to_string());
    let query = format_search_state(&state);
    let target = if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    };

    if let Ok(history) = window.history() {
        let data = web_sys::wasm_bindgen::JsValue::NULL;
        let _ = history.replace_state_with_url(&data, "", Some(&target));
    }
}

#[cfg(not(feature = "web"))]
fn write_search_state(_: SearchUrlState) {}

#[component]
pub fn SearchPage() -> Element {
    let state = read_search_state();

    use_context_provider(|| SearchUrlSync(EventHandler::new(write_search_state)));

    rsx! {
        Search {
            initial_query: state.query.clone(),
            initial_artist: state.artist.clone(),
            initial_release: state.release.clone(),
            initial_search_type: state.search_type.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_search_state, parse_search_state};
    use ui::SearchUrlState;

    #[test]
    fn parses_the_search_state_from_a_query_string() {
        let state = parse_search_state("?q=Maritime&artist=Vela%20Norte&release=9c4d&type=track");

        assert_eq!(state.query, "Maritime");
        assert_eq!(state.artist.as_deref(), Some("Vela Norte"));
        assert_eq!(state.release.as_deref(), Some("9c4d"));
        assert_eq!(state.search_type.as_deref(), Some("track"));
    }

    #[test]
    fn round_trips_values_with_query_delimiters() {
        for query in ["Maritime & Tide", "a=b", "Café 100%", "AC/DC"] {
            let original = SearchUrlState {
                query: query.to_string(),
                artist: Some("Simon & Garfunkel".to_string()),
                release: Some("id=1".to_string()),
                search_type: Some("album".to_string()),
            };
            let reparsed = parse_search_state(&format_search_state(&original));

            assert_eq!(reparsed, original, "failed to round-trip {query:?}");
        }
    }

    #[test]
    fn omits_empty_values() {
        let state = SearchUrlState {
            query: "  ".to_string(),
            artist: Some(String::new()),
            release: None,
            search_type: None,
        };

        assert!(format_search_state(&state).is_empty());
    }
}
