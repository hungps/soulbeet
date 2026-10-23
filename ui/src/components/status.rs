use dioxus::prelude::*;
use shared::media_server::MediaServerKind;
use shared::system::{MediaServerStatus, SystemHealth};

#[component]
pub fn SystemStatus(
    health: SystemHealth,
    media_status: MediaServerStatus,
    media_server: MediaServerKind,
) -> Element {
    let name = media_server.display_name().to_uppercase();
    let hint = match media_status {
        MediaServerStatus::Connected | MediaServerStatus::MissingReportRealPath => None,
        MediaServerStatus::InvalidCredentials => Some(format!(
            "Your account was not found in {}",
            media_server.display_name()
        )),
        MediaServerStatus::Unknown => Some(format!(
            "{} credentials not configured",
            media_server.display_name()
        )),
        MediaServerStatus::Offline => None,
    };

    let user_not_linked = matches!(
        media_status,
        MediaServerStatus::InvalidCredentials | MediaServerStatus::Unknown
    );

    rsx! {
      div { class: "flex justify-center gap-6 text-xs font-mono text-gray-500 flex-wrap",
        span { class: "flex items-center gap-2",
          span {
            class: format!(
                "w-2 h-2 rounded-full {}",
                if health.downloader_online { "bg-beet-leaf animate-pulse" } else { "bg-red-500" },
            ),
          }
          if health.downloader_online {
            "DOWNLOADER ONLINE"
          } else {
            "DOWNLOADER OFFLINE"
          }
        }
        span { class: "flex items-center gap-2",
          span {
            class: format!(
                "w-2 h-2 rounded-full {}",
                if health.beets_ready { "bg-beet-accent" } else { "bg-red-500" },
            ),
          }
          if health.beets_ready {
            "BEETS READY"
          } else {
            "BEETS MISSING"
          }
        }
        span {
          class: "flex items-center gap-2",
          title: hint.unwrap_or_default(),
          span {
            class: format!(
                "w-2 h-2 rounded-full {}",
                if health.media_server_online { "bg-blue-400 animate-pulse" } else { "bg-red-500" },
            ),
          }
          if health.media_server_online {
            "{name} ONLINE"
          } else if user_not_linked {
            "{name} NOT LINKED"
          } else {
            "{name} OFFLINE"
          }
        }
      }
    }
}
