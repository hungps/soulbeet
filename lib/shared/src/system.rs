use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum MediaServerStatus {
    /// Credentials verified against the media server, all features available.
    Connected,
    /// The media server rejected the credentials. Local auth succeeded but
    /// media-server features (discovery playlists, rating sync) won't work
    /// until the user logs in with matching credentials.
    InvalidCredentials,
    /// The media server was unreachable at login time. Existing token may still work.
    Offline,
    /// ReportRealPath is not enabled on the Soulbeet player in Navidrome.
    /// Path resolution and auto-delete features will not work correctly.
    MissingReportRealPath,
    /// No media-server auth has been attempted yet (fresh account).
    #[default]
    Unknown,
}

impl MediaServerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Offline => "offline",
            Self::MissingReportRealPath => "missing_report_real_path",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected | Self::MissingReportRealPath)
    }
}

impl std::fmt::Display for MediaServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MediaServerStatus {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "connected" => Ok(Self::Connected),
            "invalid_credentials" => Ok(Self::InvalidCredentials),
            "offline" => Ok(Self::Offline),
            "missing_report_real_path" => Ok(Self::MissingReportRealPath),
            _ => Ok(Self::Unknown),
        }
    }
}

impl From<&str> for MediaServerStatus {
    fn from(s: &str) -> Self {
        s.parse().unwrap_or(Self::Unknown)
    }
}

impl From<String> for MediaServerStatus {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SystemHealth {
    pub downloader_online: bool,
    pub beets_ready: bool,
    pub media_server_online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackendInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AvailableBackends {
    pub metadata: Vec<BackendInfo>,
    pub download: Vec<BackendInfo>,
    pub importer: Vec<BackendInfo>,
}
