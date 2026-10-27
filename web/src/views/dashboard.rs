use dioxus::prelude::*;
use ui::dashboard::{
    DashboardTab, DashboardTabs, DeletionHistoryTab, LibraryFocus, LibraryTab, StatsOverview,
};
use ui::discovery::DiscoveryOverview;

use crate::Route;

/// The dashboard tab as it travels in the URL. It defaults to the overview, so
/// a bare `/dashboard` and every tab share one canonical form the router will
/// not rewrite (it replaces the URL whenever a route's own form differs, which
/// used to drop the tab on every reload).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DashboardTabQuery(pub String);

impl Default for DashboardTabQuery {
    fn default() -> Self {
        Self(DashboardTab::Overview.key().to_string())
    }
}

impl std::str::FromStr for DashboardTabQuery {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(value.to_string()))
    }
}

impl std::fmt::Display for DashboardTabQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn parse_library_focus(query: &str) -> LibraryFocus {
    let query = query.strip_prefix('?').unwrap_or(query);
    let mut focus = LibraryFocus::default();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        let value = value.into_owned();
        if value.is_empty() {
            continue;
        }
        match key.as_ref() {
            "release" => focus.release = Some(value),
            "artist" => focus.artist = Some(value),
            "album" => focus.album = Some(value),
            _ => {}
        }
    }
    focus
}

#[cfg(feature = "web")]
fn read_library_focus() -> LibraryFocus {
    let Some(window) = web_sys::window() else {
        return LibraryFocus::default();
    };
    parse_library_focus(&window.location().search().unwrap_or_default())
}

#[cfg(not(feature = "web"))]
fn read_library_focus() -> LibraryFocus {
    LibraryFocus::default()
}

#[component]
pub fn DashboardPage(tab: DashboardTabQuery) -> Element {
    let navigator = use_navigator();
    let mut library_focus = use_signal(read_library_focus);

    let active_tab = DashboardTab::from_key(&tab.0);

    let change_tab = move |next: DashboardTab| {
        if next != DashboardTab::Library {
            library_focus.set(LibraryFocus::default());
        }
        navigator.replace(Route::DashboardPage {
            tab: DashboardTabQuery(next.key().to_string()),
        });
    };

    rsx! {
        div { class: "fixed top-1/4 -left-10 w-64 h-64 bg-blue-500/10 rounded-full blur-[100px] pointer-events-none max-md:hidden" }
        div { class: "fixed bottom-1/4 -right-10 w-64 h-64 bg-beet-leaf/10 rounded-full blur-[100px] pointer-events-none max-md:hidden" }

        div { class: "space-y-6 text-white w-full max-w-5xl z-10 mx-auto",
            div { class: "text-center mb-6",
                h1 { class: "text-4xl font-bold text-beet-accent mb-2 font-display",
                    "Dashboard"
                }
            }

            div { class: "flex justify-center",
                DashboardTabs {
                    active: active_tab,
                    on_change: change_tab,
                }
            }

            div { class: "pt-6",
                match active_tab {
                    DashboardTab::Overview => rsx! { OverviewTab {} },
                    DashboardTab::Library => rsx! { LibraryTab { focus: library_focus() } },
                    DashboardTab::History => rsx! { DeletionHistoryTab {} },
                    DashboardTab::Discovery => rsx! { DiscoveryOverview {} },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn the_tab_defaults_to_the_overview_in_the_url() {
        assert_eq!(
            DashboardTabQuery::default().to_string(),
            "overview",
            "a bare /dashboard and the overview tab are one url"
        );
        assert_eq!(
            DashboardTabQuery::from_str("library").unwrap(),
            DashboardTabQuery("library".to_string())
        );
        assert_eq!(
            DashboardTab::from_key(&DashboardTabQuery::from_str("history").unwrap().0),
            DashboardTab::History
        );
        assert_eq!(
            DashboardTab::from_key(&DashboardTabQuery::from_str("unknown").unwrap().0),
            DashboardTab::Overview
        );
    }

    #[test]
    fn parses_the_library_focus_from_the_url() {
        assert_eq!(
            parse_library_focus("?tab=library&release=abc&artist=Ed+Sheeran&album=%C3%B7"),
            LibraryFocus {
                release: Some("abc".to_string()),
                artist: Some("Ed Sheeran".to_string()),
                album: Some("÷".to_string()),
            }
        );
        assert!(parse_library_focus("?tab=library").is_empty());
    }
}

#[component]
fn OverviewTab() -> Element {
    let stats_resource = use_resource(|| async { api::get_library_stats().await });
    let binding = stats_resource.read();

    match &*binding {
        Some(Ok(s)) => {
            let s = s.clone();
            rsx! { StatsOverview { stats: s } }
        }
        Some(Err(e)) => rsx! {
            div { class: "text-center text-gray-500 font-mono max-w-xl mx-auto",
                p { class: "text-red-400", "Failed to load library stats: {e}" }
                p { class: "text-xs mt-2",
                    "Check Settings > Media Server and make sure the server is reachable from this network."
                }
            }
        },
        None => rsx! {
            div { class: "text-center text-gray-400 font-mono animate-pulse",
                "Loading stats..."
            }
        },
    }
}
