use serde::{Deserialize, Serialize};

use crate::system::MediaServerStatus;

/// The media server a user is linked to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MediaServerKind {
    #[default]
    Navidrome,
    Plex,
}

impl MediaServerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Navidrome => "navidrome",
            Self::Plex => "plex",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Navidrome => "Navidrome",
            Self::Plex => "Plex",
        }
    }
}

impl std::fmt::Display for MediaServerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MediaServerKind {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "plex" => Ok(Self::Plex),
            "navidrome" => Ok(Self::Navidrome),
            _ => Err(()),
        }
    }
}

impl From<&str> for MediaServerKind {
    fn from(s: &str) -> Self {
        s.parse().unwrap_or_default()
    }
}

/// A track as exposed by a media server, normalised across providers.
/// Ratings use Soulbeet's 1-5 star scale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaTrack {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub duration_secs: Option<u32>,
    /// Path as reported by the media server. May be relative (Navidrome) or
    /// absolute inside the server's container (Plex).
    pub path: Option<String>,
    pub user_rating: Option<u8>,
    /// Average across all users, when the server exposes it (Navidrome only).
    pub average_rating: Option<f64>,
    pub play_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAlbum {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub song_count: Option<u32>,
    pub genre: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AuthServers {
    pub navidrome: bool,
    pub plex: bool,
}

/// A Plex Media Server exposed by plex.tv for the signed-in account.
/// One entry is produced per usable connection so the settings UI can offer
/// LAN, WAN and relay addresses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlexServerOption {
    pub name: String,
    pub machine_identifier: String,
    pub address: String,
    pub port: u16,
    pub secure: bool,
    pub local: bool,
    pub relay: bool,
    #[serde(default)]
    pub uri: String,
}

impl PlexServerOption {
    pub fn url(&self, secure: bool) -> String {
        if secure == self.secure && !self.uri.is_empty() {
            return self.uri.clone();
        }
        let host = if self.address.contains(':') && !self.address.starts_with('[') {
            format!("[{}]", self.address)
        } else {
            self.address.clone()
        };
        format!(
            "{}://{}:{}",
            if secure { "https" } else { "http" },
            host,
            self.port
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlexPin {
    pub id: String,
    pub code: String,
    pub auth_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaPathMapping {
    pub id: String,
    pub kind: MediaServerKind,
    pub local_path: String,
    pub remote_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaServerView {
    pub kind: MediaServerKind,
    pub url: Option<String>,
    pub secure: bool,
    pub account_name: Option<String>,
    pub server_name: Option<String>,
    pub server_id: Option<String>,
    pub linked: bool,
    pub status: MediaServerStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaServerSettings {
    pub active: MediaServerKind,
    pub navidrome: Option<MediaServerView>,
    pub plex: Option<MediaServerView>,
    pub plex_servers: Vec<PlexServerOption>,
    pub mappings: Vec<MediaPathMapping>,
}

impl AuthServers {
    pub fn any(&self) -> bool {
        self.navidrome || self.plex
    }

    pub fn default_kind(&self) -> MediaServerKind {
        if self.navidrome {
            MediaServerKind::Navidrome
        } else {
            MediaServerKind::Plex
        }
    }

    pub fn supports(&self, kind: MediaServerKind) -> bool {
        match kind {
            MediaServerKind::Navidrome => self.navidrome,
            MediaServerKind::Plex => self.plex,
        }
    }
}
