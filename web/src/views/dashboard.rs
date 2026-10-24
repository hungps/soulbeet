use dioxus::prelude::*;
use ui::dashboard::{DashboardTab, DashboardTabs, DeletionHistoryTab, LibraryTab, StatsOverview};
use ui::discovery::DiscoveryOverview;

#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn parse_dashboard_tab(query: &str) -> DashboardTab {
    let query = query.strip_prefix('?').unwrap_or(query);
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "tab" {
            return DashboardTab::from_key(&value);
        }
    }
    DashboardTab::default()
}

#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn dashboard_tab_url(tab: DashboardTab) -> String {
    format!("/dashboard?tab={}", tab.key())
}

#[cfg(feature = "web")]
fn read_dashboard_tab() -> DashboardTab {
    let Some(window) = web_sys::window() else {
        return DashboardTab::default();
    };
    parse_dashboard_tab(&window.location().search().unwrap_or_default())
}

#[cfg(not(feature = "web"))]
fn read_dashboard_tab() -> DashboardTab {
    DashboardTab::default()
}

#[cfg(feature = "web")]
fn write_dashboard_tab(tab: DashboardTab) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let target = dashboard_tab_url(tab);
    if let Ok(history) = window.history() {
        let data = web_sys::wasm_bindgen::JsValue::NULL;
        let _ = history.replace_state_with_url(&data, "", Some(&target));
    }
}

#[cfg(not(feature = "web"))]
fn write_dashboard_tab(_: DashboardTab) {}

#[component]
pub fn DashboardPage() -> Element {
    let mut active_tab = use_signal(read_dashboard_tab);

    let change_tab = move |tab: DashboardTab| {
        active_tab.set(tab);
        write_dashboard_tab(tab);
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
                    active: active_tab(),
                    on_change: change_tab,
                }
            }

            div { class: "pt-6",
                match active_tab() {
                    DashboardTab::Overview => rsx! { OverviewTab {} },
                    DashboardTab::Library => rsx! { LibraryTab {} },
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

    #[test]
    fn parses_the_dashboard_tab_from_the_url() {
        assert_eq!(parse_dashboard_tab("?tab=library"), DashboardTab::Library);
        assert_eq!(parse_dashboard_tab("tab=history"), DashboardTab::History);
        assert_eq!(
            parse_dashboard_tab("?tab=discovery&x=1"),
            DashboardTab::Discovery
        );
        assert_eq!(parse_dashboard_tab(""), DashboardTab::Overview);
        assert_eq!(parse_dashboard_tab("?tab=unknown"), DashboardTab::Overview);
    }

    #[test]
    fn writes_a_stable_dashboard_url() {
        assert_eq!(
            dashboard_tab_url(DashboardTab::Library),
            "/dashboard?tab=library"
        );
        assert_eq!(
            dashboard_tab_url(DashboardTab::Overview),
            "/dashboard?tab=overview"
        );
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
