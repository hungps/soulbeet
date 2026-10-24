use dioxus::prelude::*;
use shared::download::AutoDownloadEvent;

#[derive(Clone, Copy)]
pub struct SearchReset(pub Signal<u32>);

#[derive(Clone, Copy)]
pub struct SearchPrefill(pub Signal<Option<(String, String)>>);

#[derive(Clone, PartialEq, Default, Debug)]
pub struct SearchUrlState {
    pub query: String,
    pub artist: Option<String>,
    pub release: Option<String>,
}

#[derive(Clone, Copy)]
pub struct SearchUrlSync(pub EventHandler<SearchUrlState>);

#[derive(Clone, Copy)]
pub struct AutoDownloadSignal(pub Signal<Option<AutoDownloadEvent>>);
