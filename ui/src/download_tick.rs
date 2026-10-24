use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DownloadTick(pub Signal<u32>);
