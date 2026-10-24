use crate::auth::AuthResponse;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(feature = "server")]
use shared::media_server::MediaServerView;
use shared::media_server::{
    MediaPathMapping, MediaServerKind, MediaServerSettings, PlexPin, PlexServerOption,
};
#[cfg(feature = "server")]
use shared::system::MediaServerStatus;

#[cfg(feature = "server")]
use crate::{
    crypto,
    models::{
        media_server::{self, MediaPathMappingRow, MediaServerConfig},
        user::User,
        user_settings::UserSettings,
    },
    server_fns::server_error,
    services::{evict_media_server, media_server_for_user, plex_client_id},
    AuthSession,
};

#[cfg(feature = "server")]
use tower_cookies::Cookies;

#[cfg(feature = "server")]
use dioxus::logger::tracing::{info, warn};

#[cfg(feature = "server")]
use soulbeet::plex::PlexClientBuilder;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlexLoginPoll {
    Pending,
    Complete(AuthResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlexLinkPoll {
    Pending,
    Complete {
        account_name: String,
        servers: Vec<PlexServerOption>,
    },
}

#[cfg(feature = "server")]
fn best_server(servers: &[PlexServerOption]) -> Option<&PlexServerOption> {
    servers
        .iter()
        .filter(|s| s.local && !s.relay)
        .min_by_key(|s| if s.secure { 1 } else { 0 })
        .or_else(|| {
            servers
                .iter()
                .filter(|s| !s.relay)
                .min_by_key(|s| if s.secure { 1 } else { 0 })
        })
        .or_else(|| servers.first())
}

#[cfg(feature = "server")]
#[derive(Debug, Clone)]
struct PlexConnection {
    url: String,
    secure: bool,
    server_id: Option<String>,
    server_name: Option<String>,
    status: MediaServerStatus,
}

#[cfg(feature = "server")]
async fn resolve_plex_connection(
    client_id: &str,
    token: &str,
    existing: Option<PlexConnection>,
    servers: &[PlexServerOption],
) -> Option<PlexConnection> {
    let mut failed_status = None;
    if let Some(connection) = &existing {
        let (status, error) = ping_plex(&connection.url, token, client_id).await;
        match error {
            None => {
                return Some(PlexConnection {
                    status,
                    ..connection.clone()
                })
            }
            Some(error) => {
                failed_status = Some(status);
                warn!(
                    "Saved Plex connection {} is unreachable: {}; probing plex.tv",
                    connection.url, error
                );
            }
        }
    }

    if let Some(reachable) =
        soulbeet::plex::select_reachable_server(client_id, token, servers).await
    {
        let option = reachable.option;
        info!(
            "Selected Plex connection {} ({:.0}ms)",
            reachable.url,
            reachable.latency.as_secs_f64() * 1000.0
        );
        return Some(PlexConnection {
            url: reachable.url,
            secure: option.secure,
            server_id: Some(option.machine_identifier),
            server_name: Some(option.name),
            status: MediaServerStatus::Connected,
        });
    }

    if let Some(mut connection) = existing {
        connection.status = failed_status.unwrap_or(connection.status);
        return Some(connection);
    }

    best_server(servers).map(|option| PlexConnection {
        url: server_url(option, option.secure),
        secure: option.secure,
        server_id: Some(option.machine_identifier.clone()),
        server_name: Some(option.name.clone()),
        status: MediaServerStatus::Offline,
    })
}

#[cfg(feature = "server")]
fn server_url(server: &PlexServerOption, secure: bool) -> String {
    server.url(secure)
}

#[cfg(feature = "server")]
fn normalize_url(url: &str, secure: bool) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("{}://{}", if secure { "https" } else { "http" }, trimmed)
    }
}

#[cfg(feature = "server")]
fn plex_status(error: &soulbeet::error::SoulseekError) -> MediaServerStatus {
    match error {
        soulbeet::error::SoulseekError::Api { status: 401, .. } => {
            MediaServerStatus::InvalidCredentials
        }
        _ => MediaServerStatus::Offline,
    }
}

#[cfg(feature = "server")]
async fn ping_plex(
    url: &str,
    token: &str,
    client_id: &str,
) -> (MediaServerStatus, Option<soulbeet::error::SoulseekError>) {
    let client = match PlexClientBuilder::new()
        .base_url(url)
        .token(token)
        .client_id(client_id)
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            return (
                MediaServerStatus::Offline,
                Some(soulbeet::error::SoulseekError::Api {
                    status: 0,
                    message: e.to_string(),
                }),
            )
        }
    };

    match client.ping().await {
        Ok(()) => (MediaServerStatus::Connected, None),
        Err(e) => (plex_status(&e), Some(e)),
    }
}

#[cfg(feature = "server")]
async fn unique_username(base: &str, plex_id: &str) -> Result<String, String> {
    let base = if base.trim().is_empty() {
        format!("plex-{}", plex_id)
    } else {
        base.trim().to_string()
    };

    if User::get_by_username(&base).await?.is_none() {
        return Ok(base);
    }

    for suffix in 2..1000u32 {
        let candidate = format!("{} ({})", base, suffix);
        if User::get_by_username(&candidate).await?.is_none() {
            return Ok(candidate);
        }
    }

    Err("Could not allocate a local username for the Plex account".to_string())
}

#[cfg(feature = "server")]
fn view_for(kind: MediaServerKind, config: Option<&MediaServerConfig>) -> MediaServerView {
    MediaServerView {
        kind,
        url: config.and_then(|c| c.url.clone()),
        secure: config.map(|c| c.secure).unwrap_or(false),
        account_name: config.and_then(|c| c.account_name.clone()),
        server_name: config.and_then(|c| c.server_name.clone()),
        server_id: config.and_then(|c| c.server_id.clone()),
        linked: config.and_then(|c| c.token.as_ref()).is_some(),
        status: config
            .map(|c| MediaServerStatus::from(c.status.as_str()))
            .unwrap_or(MediaServerStatus::Unknown),
    }
}

#[cfg(feature = "server")]
async fn plex_servers_for(config: Option<&MediaServerConfig>) -> Vec<PlexServerOption> {
    let Some(config) = config else {
        return Vec::new();
    };
    let Some(token) = config.token.as_deref() else {
        return Vec::new();
    };
    let Ok(token) = crypto::decrypt(token) else {
        return Vec::new();
    };
    let Ok(client_id) = plex_client_id().await else {
        return Vec::new();
    };
    soulbeet::plex::list_servers(soulbeet::plex::DEFAULT_PLEX_TV_URL, &client_id, &token)
        .await
        .unwrap_or_default()
}

#[cfg(feature = "server")]
async fn load_settings(user_id: &str) -> Result<MediaServerSettings, String> {
    let user = User::get_by_id(user_id).await?;
    let active = MediaServerKind::from(user.media_server.as_str());
    let navidrome = MediaServerConfig::get(user_id, MediaServerKind::Navidrome).await?;
    let plex = MediaServerConfig::get(user_id, MediaServerKind::Plex).await?;
    let plex_servers = plex_servers_for(plex.as_ref()).await;
    let mappings = MediaPathMappingRow::list_for_user(user_id, None)
        .await?
        .into_iter()
        .map(MediaPathMappingRow::into_shared)
        .collect();

    Ok(MediaServerSettings {
        active,
        navidrome: Some(view_for(MediaServerKind::Navidrome, navidrome.as_ref())),
        plex: Some(view_for(MediaServerKind::Plex, plex.as_ref())),
        plex_servers,
        mappings,
    })
}

#[cfg(feature = "server")]
async fn set_active(
    user_id: &str,
    kind: MediaServerKind,
    token: Option<&str>,
    status: MediaServerStatus,
) -> Result<(), String> {
    User::set_active_media(user_id, kind.as_str(), token, status.as_str()).await?;
    media_server::reset_playlists_for_server_change(user_id).await;
    evict_media_server(user_id).await;
    Ok(())
}

#[post("/api/plex/pin")]
pub async fn create_plex_pin() -> Result<PlexPin, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let client_id = plex_client_id().await.map_err(server_error)?;
        soulbeet::plex::create_pin(soulbeet::plex::DEFAULT_PLEX_TV_URL, &client_id)
            .await
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    unreachable!()
}

#[post("/api/plex/login/poll", cookies: Cookies)]
pub async fn poll_plex_login(pin_id: String) -> Result<PlexLoginPoll, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let client_id = plex_client_id().await.map_err(server_error)?;
        let Some(plex_user) =
            soulbeet::plex::check_pin(soulbeet::plex::DEFAULT_PLEX_TV_URL, &client_id, &pin_id)
                .await
                .map_err(server_error)?
        else {
            return Ok(PlexLoginPoll::Pending);
        };

        let user =
            match MediaServerConfig::find_user_by_account(MediaServerKind::Plex, &plex_user.id)
                .await
                .map_err(server_error)?
            {
                Some(user_id) => User::get_by_id(&user_id).await.map_err(server_error)?,
                None => {
                    let username = unique_username(&plex_user.username, &plex_user.id)
                        .await
                        .map_err(server_error)?;
                    let password = uuid::Uuid::new_v4().to_string();
                    User::create(&username, &password)
                        .await
                        .map_err(server_error)?
                }
            };

        let encrypted = crypto::encrypt(&plex_user.token).map_err(server_error)?;
        let existing = MediaServerConfig::get(&user.id, MediaServerKind::Plex)
            .await
            .map_err(server_error)?;
        let existing_connection = existing.as_ref().and_then(|config| {
            config.url.clone().map(|url| PlexConnection {
                url,
                secure: config.secure,
                server_id: config.server_id.clone(),
                server_name: config.server_name.clone(),
                status: MediaServerStatus::from(config.status.as_str()),
            })
        });

        let servers = soulbeet::plex::list_servers(
            soulbeet::plex::DEFAULT_PLEX_TV_URL,
            &client_id,
            &plex_user.token,
        )
        .await
        .unwrap_or_default();
        let connection =
            resolve_plex_connection(&client_id, &plex_user.token, existing_connection, &servers)
                .await;

        let (status, saved) = match connection {
            Some(connection) => (
                connection.status,
                Some((
                    connection.url,
                    connection.secure,
                    connection.server_id,
                    connection.server_name,
                )),
            ),
            None => (MediaServerStatus::Unknown, None),
        };

        MediaServerConfig::link_account(
            &user.id,
            MediaServerKind::Plex,
            &encrypted,
            Some(&plex_user.id),
            Some(&plex_user.username),
            status.as_str(),
        )
        .await
        .map_err(server_error)?;

        if let Some((url, secure, server_id, server_name)) = saved {
            MediaServerConfig::save_server(
                &user.id,
                MediaServerKind::Plex,
                &url,
                secure,
                server_id.as_deref(),
                server_name.as_deref(),
                status.as_str(),
            )
            .await
            .map_err(server_error)?;
        }

        User::update_media(
            &user.id,
            "plex",
            Some(&encrypted),
            status.as_str(),
            Some(&plex_user.id),
        )
        .await
        .map_err(server_error)?;
        let _ = UserSettings::reset_navidrome_banner(&user.id).await;
        evict_media_server(&user.id).await;

        let response = crate::server_fns::auth::issue_session(
            user.id,
            user.username,
            MediaServerKind::Plex,
            status,
            cookies,
        )?;
        Ok(PlexLoginPoll::Complete(response))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = pin_id;
        unreachable!()
    }
}

#[post("/api/plex/link/poll", auth: AuthSession)]
pub async fn poll_plex_link(pin_id: String) -> Result<PlexLinkPoll, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let user_id = auth.0.sub.clone();
        let client_id = plex_client_id().await.map_err(server_error)?;
        let Some(plex_user) =
            soulbeet::plex::check_pin(soulbeet::plex::DEFAULT_PLEX_TV_URL, &client_id, &pin_id)
                .await
                .map_err(server_error)?
        else {
            return Ok(PlexLinkPoll::Pending);
        };

        if let Some(linked_user) =
            MediaServerConfig::find_user_by_account(MediaServerKind::Plex, &plex_user.id)
                .await
                .map_err(server_error)?
        {
            if linked_user != user_id {
                return Err(server_error(
                    "This Plex account is already linked to another user",
                ));
            }
        }

        let servers = soulbeet::plex::list_servers(
            soulbeet::plex::DEFAULT_PLEX_TV_URL,
            &client_id,
            &plex_user.token,
        )
        .await
        .unwrap_or_default();

        let existing = MediaServerConfig::get(&user_id, MediaServerKind::Plex)
            .await
            .map_err(server_error)?;
        let encrypted = crypto::encrypt(&plex_user.token).map_err(server_error)?;
        let existing_connection = existing.as_ref().and_then(|config| {
            config.url.clone().map(|url| PlexConnection {
                url,
                secure: config.secure,
                server_id: config.server_id.clone(),
                server_name: config.server_name.clone(),
                status: MediaServerStatus::from(config.status.as_str()),
            })
        });

        let connection =
            resolve_plex_connection(&client_id, &plex_user.token, existing_connection, &servers)
                .await;
        let (status, saved) = match connection {
            Some(connection) => (
                connection.status,
                Some((
                    connection.url,
                    connection.secure,
                    connection.server_id,
                    connection.server_name,
                )),
            ),
            None => (MediaServerStatus::Unknown, None),
        };

        MediaServerConfig::link_account(
            &user_id,
            MediaServerKind::Plex,
            &encrypted,
            Some(&plex_user.id),
            Some(&plex_user.username),
            status.as_str(),
        )
        .await
        .map_err(server_error)?;

        if let Some((url, secure, server_id, server_name)) = saved {
            MediaServerConfig::save_server(
                &user_id,
                MediaServerKind::Plex,
                &url,
                secure,
                server_id.as_deref(),
                server_name.as_deref(),
                status.as_str(),
            )
            .await
            .map_err(server_error)?;
        }

        User::update_media(
            &user_id,
            "plex",
            Some(&encrypted),
            status.as_str(),
            Some(&plex_user.id),
        )
        .await
        .map_err(server_error)?;
        media_server::reset_playlists_for_server_change(&user_id).await;
        evict_media_server(&user_id).await;

        info!(
            "Linked Plex account '{}' for user {}",
            plex_user.username, user_id
        );
        Ok(PlexLinkPoll::Complete {
            account_name: plex_user.username,
            servers,
        })
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (auth, pin_id);
        unreachable!()
    }
}

#[get("/api/media-server/settings", auth: AuthSession)]
pub async fn get_media_server_settings() -> Result<MediaServerSettings, ServerFnError> {
    #[cfg(feature = "server")]
    {
        load_settings(&auth.0.sub).await.map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = auth;
        unreachable!()
    }
}

#[post("/api/media-server/active", auth: AuthSession)]
pub async fn set_active_media_server(kind: MediaServerKind) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let user_id = auth.0.sub;
        let config = MediaServerConfig::get(&user_id, kind)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error(format!("{} is not linked", kind.display_name())))?;
        let token = config
            .token
            .as_deref()
            .ok_or_else(|| server_error(format!("{} is not linked", kind.display_name())))?;
        set_active(
            &user_id,
            kind,
            Some(token),
            MediaServerStatus::from(config.status.as_str()),
        )
        .await
        .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (auth, kind);
        unreachable!()
    }
}

#[post("/api/media-server/navidrome", auth: AuthSession)]
pub async fn connect_navidrome(
    url: String,
    username: String,
    password: Option<String>,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        use soulbeet::NavidromeClientBuilder;

        let url = normalize_url(&url, false);
        let username = username.trim().to_string();
        if username.is_empty() {
            return Err(server_error("Navidrome username cannot be empty"));
        }

        let existing = MediaServerConfig::get(&auth.0.sub, MediaServerKind::Navidrome)
            .await
            .map_err(server_error)?;
        let password = match password.filter(|p| !p.is_empty()) {
            Some(password) => password,
            None => existing
                .as_ref()
                .and_then(|c| c.token.as_deref())
                .map(crypto::decrypt)
                .transpose()
                .map_err(server_error)?
                .ok_or_else(|| server_error("Navidrome password is required"))?,
        };

        let client = NavidromeClientBuilder::new()
            .base_url(&url)
            .username(&username)
            .password(&password)
            .build()
            .map_err(server_error)?;

        if let Err(e) = client.ping().await {
            let status = if e.to_string().contains("Navidrome request failed") {
                MediaServerStatus::Offline
            } else {
                MediaServerStatus::InvalidCredentials
            };
            let _ = MediaServerConfig::update_status(
                &auth.0.sub,
                MediaServerKind::Navidrome,
                status.as_str(),
            )
            .await;
            return Err(server_error(e.to_string()));
        }

        let encrypted = crypto::encrypt(&password).map_err(server_error)?;
        MediaServerConfig::upsert_navidrome(
            &auth.0.sub,
            &url,
            false,
            &encrypted,
            &username,
            MediaServerStatus::Connected.as_str(),
        )
        .await
        .map_err(server_error)?;
        set_active(
            &auth.0.sub,
            MediaServerKind::Navidrome,
            Some(&encrypted),
            MediaServerStatus::Connected,
        )
        .await
        .map_err(server_error)?;
        let _ = UserSettings::reset_navidrome_banner(&auth.0.sub).await;
        Ok(())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (auth, url, username, password);
        unreachable!()
    }
}

#[post("/api/media-server/navidrome/disconnect", auth: AuthSession)]
pub async fn disconnect_navidrome() -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let user_id = auth.0.sub;
        let user = User::get_by_id(&user_id).await.map_err(server_error)?;
        MediaServerConfig::delete(&user_id, MediaServerKind::Navidrome)
            .await
            .map_err(server_error)?;

        if MediaServerKind::from(user.media_server.as_str()) == MediaServerKind::Navidrome {
            if let Some(plex) = MediaServerConfig::get(&user_id, MediaServerKind::Plex)
                .await
                .map_err(server_error)?
            {
                set_active(
                    &user_id,
                    MediaServerKind::Plex,
                    plex.token.as_deref(),
                    MediaServerStatus::from(plex.status.as_str()),
                )
                .await
                .map_err(server_error)?;
            } else {
                set_active(
                    &user_id,
                    MediaServerKind::Navidrome,
                    None,
                    MediaServerStatus::Unknown,
                )
                .await
                .map_err(server_error)?;
            }
        }

        evict_media_server(&user_id).await;
        Ok(())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = auth;
        unreachable!()
    }
}

#[post("/api/media-server/plex", auth: AuthSession)]
pub async fn save_plex_config(
    url: String,
    secure: bool,
    server_id: Option<String>,
    server_name: Option<String>,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let user_id = auth.0.sub;
        let config = MediaServerConfig::get(&user_id, MediaServerKind::Plex)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Log in with Plex first"))?;
        let token = config
            .token
            .as_deref()
            .ok_or_else(|| server_error("Log in with Plex first"))?;
        let token = crypto::decrypt(token).map_err(server_error)?;
        let client_id = plex_client_id().await.map_err(server_error)?;
        let url = normalize_url(&url, secure);
        let (status, error) = ping_plex(&url, &token, &client_id).await;
        if let Some(error) = error {
            warn!("Plex server rejected the configured URL: {}", error);
        }

        MediaServerConfig::save_server(
            &user_id,
            MediaServerKind::Plex,
            &url,
            secure,
            server_id.as_deref().filter(|v| !v.is_empty()),
            server_name.as_deref().filter(|v| !v.is_empty()),
            status.as_str(),
        )
        .await
        .map_err(server_error)?;
        set_active(&user_id, MediaServerKind::Plex, Some(&token), status)
            .await
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (url, secure, server_id, server_name, auth);
        unreachable!()
    }
}

#[post("/api/media-server/plex/auto", auth: AuthSession)]
pub async fn auto_select_plex_server() -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let user_id = auth.0.sub;
        let config = MediaServerConfig::get(&user_id, MediaServerKind::Plex)
            .await
            .map_err(server_error)?
            .ok_or_else(|| server_error("Log in with Plex first"))?;
        let token = config
            .token
            .as_deref()
            .ok_or_else(|| server_error("Log in with Plex first"))?;
        let token = crypto::decrypt(token).map_err(server_error)?;
        let client_id = plex_client_id().await.map_err(server_error)?;
        let servers =
            soulbeet::plex::list_servers(soulbeet::plex::DEFAULT_PLEX_TV_URL, &client_id, &token)
                .await
                .map_err(|e| server_error(e.to_string()))?;
        let reachable = soulbeet::plex::select_reachable_server(&client_id, &token, &servers)
            .await
            .ok_or_else(|| server_error("No Plex connection is reachable from this network"))?;
        let option = reachable.option;

        MediaServerConfig::save_server(
            &user_id,
            MediaServerKind::Plex,
            &reachable.url,
            option.secure,
            Some(&option.machine_identifier),
            Some(&option.name),
            MediaServerStatus::Connected.as_str(),
        )
        .await
        .map_err(server_error)?;
        set_active(
            &user_id,
            MediaServerKind::Plex,
            config.token.as_deref(),
            MediaServerStatus::Connected,
        )
        .await
        .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = auth;
        unreachable!()
    }
}

#[post("/api/media-server/plex/disconnect", auth: AuthSession)]
pub async fn disconnect_plex() -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let user_id = auth.0.sub;
        let user = User::get_by_id(&user_id).await.map_err(server_error)?;
        let config = MediaServerConfig::get(&user_id, MediaServerKind::Plex)
            .await
            .map_err(server_error)?;
        MediaServerConfig::delete(&user_id, MediaServerKind::Plex)
            .await
            .map_err(server_error)?;

        if config
            .as_ref()
            .and_then(|c| c.account_id.as_deref())
            .is_some_and(|id| user.media_account_id.as_deref() == Some(id))
        {
            User::clear_media_account(&user_id)
                .await
                .map_err(server_error)?;
        }

        if MediaServerKind::from(user.media_server.as_str()) == MediaServerKind::Plex {
            if let Some(navidrome) = MediaServerConfig::get(&user_id, MediaServerKind::Navidrome)
                .await
                .map_err(server_error)?
            {
                set_active(
                    &user_id,
                    MediaServerKind::Navidrome,
                    navidrome.token.as_deref(),
                    MediaServerStatus::from(navidrome.status.as_str()),
                )
                .await
                .map_err(server_error)?;
            } else {
                set_active(
                    &user_id,
                    MediaServerKind::Plex,
                    None,
                    MediaServerStatus::Unknown,
                )
                .await
                .map_err(server_error)?;
            }
        }

        evict_media_server(&user_id).await;
        Ok(())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = auth;
        unreachable!()
    }
}

#[get("/api/media-server/plex/servers", auth: AuthSession)]
pub async fn list_plex_servers() -> Result<Vec<PlexServerOption>, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let config = MediaServerConfig::get(&auth.0.sub, MediaServerKind::Plex)
            .await
            .map_err(server_error)?;
        Ok(plex_servers_for(config.as_ref()).await)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = auth;
        unreachable!()
    }
}

#[get("/api/media-server/mappings", auth: AuthSession)]
pub async fn get_path_mappings(
    kind: Option<MediaServerKind>,
) -> Result<Vec<MediaPathMapping>, ServerFnError> {
    #[cfg(feature = "server")]
    {
        MediaPathMappingRow::list_for_user(&auth.0.sub, kind)
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(MediaPathMappingRow::into_shared)
                    .collect()
            })
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (auth, kind);
        unreachable!()
    }
}

#[post("/api/media-server/mappings", auth: AuthSession)]
pub async fn add_path_mapping(
    kind: MediaServerKind,
    local_path: String,
    remote_path: String,
) -> Result<MediaPathMapping, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let local_path = local_path.trim().trim_end_matches('/').to_string();
        let remote_path = remote_path.trim().trim_end_matches('/').to_string();
        if local_path.is_empty() || remote_path.is_empty() {
            return Err(server_error("Both paths are required"));
        }
        MediaPathMappingRow::create(&auth.0.sub, kind, &local_path, &remote_path)
            .await
            .map(MediaPathMappingRow::into_shared)
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (auth, kind, local_path, remote_path);
        unreachable!()
    }
}

#[post("/api/media-server/mappings/update", auth: AuthSession)]
pub async fn update_path_mapping(
    id: String,
    local_path: String,
    remote_path: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        MediaPathMappingRow::update(
            &id,
            &auth.0.sub,
            local_path.trim().trim_end_matches('/'),
            remote_path.trim().trim_end_matches('/'),
        )
        .await
        .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (auth, id, local_path, remote_path);
        unreachable!()
    }
}

#[post("/api/media-server/mappings/delete", auth: AuthSession)]
pub async fn delete_path_mapping(id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        MediaPathMappingRow::delete(&id, &auth.0.sub)
            .await
            .map_err(server_error)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (auth, id);
        unreachable!()
    }
}

#[cfg(feature = "server")]
pub async fn trigger_media_scan(user_id: &str, local_path: &std::path::Path) {
    let Ok(server) = media_server_for_user(user_id).await else {
        return;
    };
    if server.kind() != MediaServerKind::Plex {
        return;
    }

    let local = local_path.to_string_lossy().to_string();
    let paths: Vec<String> = media_server::local_to_remote(user_id, server.kind(), &local)
        .await
        .ok()
        .flatten()
        .into_iter()
        .collect();

    if let Err(e) = server.start_scan_paths(&paths).await {
        warn!(
            "Failed to trigger {} scan after import: {}",
            server.kind(),
            e
        );
    }
}
