use crate::auth::AuthResponse;
use dioxus::prelude::*;
use shared::media_server::{AuthServers, MediaServerKind};

#[cfg(feature = "server")]
use crate::{
    auth, models,
    server_fns::{server_error, unauthorized_error},
    AuthSession,
};

#[cfg(feature = "server")]
use tower_cookies::{
    cookie::{time, SameSite},
    Cookie, Cookies,
};

pub const AUTH_COOKIE_NAME: &str = "auth_token";

/// Helper to configure the auth cookie consistently
#[cfg(feature = "server")]
fn build_auth_cookie(token: String) -> Cookie<'static> {
    use crate::auth::EXPIRATION_DAYS;

    let mut cookie = Cookie::new(AUTH_COOKIE_NAME, token);
    cookie.set_path("/");
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_expires(time::OffsetDateTime::now_utc() + time::Duration::days(EXPIRATION_DAYS));
    cookie
}

/// Which media servers this instance is configured for. Public: the login
/// page needs it before there is a session.
#[get("/api/auth/servers")]
pub async fn get_auth_servers() -> Result<AuthServers, ServerFnError> {
    #[cfg(feature = "server")]
    {
        Ok(crate::services::available_media_servers().await)
    }
    #[cfg(not(feature = "server"))]
    Ok(AuthServers::default())
}

#[post("/api/auth/register")]
pub async fn register(username: String, password: String) -> Result<(), ServerFnError> {
    models::user::User::create(&username, &password)
        .await
        .map_err(server_error)
        .map(|_| ())
}

/// Categorized result of a Navidrome ping attempt.
#[cfg(feature = "server")]
enum NavidromeAuthResult {
    /// Credentials accepted by Navidrome.
    Success,
    /// Navidrome responded but credentials were wrong (Subsonic error code 40).
    AuthFailed,
    /// Navidrome could not be reached (network / timeout / circuit breaker).
    Unreachable,
    /// The user has no saved Navidrome URL: authenticate locally instead.
    NotConfigured,
}

/// Try to authenticate against Navidrome with the given credentials.
///
/// Builds a temporary NavidromeClient and calls `ping()`. The error type from
/// soulbeet lets us distinguish auth rejection (Subsonic error codes) from
/// network failures (status 408, 503, or request-level errors).
#[cfg(feature = "server")]
async fn try_navidrome_auth(
    url: Option<&str>,
    username: &str,
    password: &str,
) -> NavidromeAuthResult {
    use soulbeet::NavidromeClientBuilder;

    let Some(url) = url else {
        return NavidromeAuthResult::NotConfigured;
    };

    let client = match NavidromeClientBuilder::new()
        .base_url(url)
        .username(username)
        .password(password)
        .build()
    {
        Ok(c) => c,
        Err(_) => return NavidromeAuthResult::Unreachable,
    };

    match client.ping().await {
        Ok(()) => NavidromeAuthResult::Success,
        Err(e) => {
            let err_str = e.to_string();
            // Network-level failures include timeouts (408), service unavailable
            // (503), and connection errors. Subsonic auth failures produce API
            // error code 40 ("Wrong username or password") with a lower status.
            if err_str.contains("Navidrome request failed")
                || err_str.contains("Circuit breaker open")
            {
                tracing::warn!("Navidrome unreachable during login: {}", err_str);
                NavidromeAuthResult::Unreachable
            } else {
                tracing::info!("Navidrome auth failed for {}: {}", username, err_str);
                NavidromeAuthResult::AuthFailed
            }
        }
    }
}

#[post("/api/auth/login", cookies: Cookies)]
pub async fn login(
    username: String,
    password: String,
    server: Option<MediaServerKind>,
) -> Result<AuthResponse, ServerFnError> {
    use crate::services::available_media_servers;

    let available = available_media_servers().await;

    // Local-account login: authenticate against the Argon2 hash only. Also
    // covers instance-wide local mode, where no media server is configured.
    if server.is_none() || !available.any() {
        use models::user::User;
        use shared::system::MediaServerStatus;

        let user = User::verify(&username, &password)
            .await
            .map_err(|_| unauthorized_error("Invalid username or password"))?;
        return issue_session(
            user.id,
            user.username,
            MediaServerKind::from(user.media_server.as_str()),
            MediaServerStatus::from(user.media_status),
            cookies,
        );
    }

    let kind = server.unwrap_or_else(|| available.default_kind());
    if !available.supports(kind) {
        return Err(unauthorized_error(format!(
            "{} is not configured on this server",
            kind.display_name()
        )));
    }

    match kind {
        MediaServerKind::Navidrome => login_navidrome(username, password, cookies).await,
        MediaServerKind::Plex => login_plex(username, password, cookies).await,
    }
}

#[cfg(feature = "server")]
async fn login_navidrome(
    username: String,
    password: String,
    cookies: Cookies,
) -> Result<AuthResponse, ServerFnError> {
    use crate::crypto;
    use crate::models::media_server::MediaServerConfig;
    use crate::services::evict_media_server;
    use models::user::User;
    use shared::system::MediaServerStatus;

    let stored_url = match User::get_by_username(&username)
        .await
        .map_err(server_error)?
    {
        Some(user) => MediaServerConfig::get(&user.id, MediaServerKind::Navidrome)
            .await
            .map_err(server_error)?
            .and_then(|c| c.url),
        None => None,
    };

    match try_navidrome_auth(stored_url.as_deref(), &username, &password).await {
        NavidromeAuthResult::Success => {
            let user = match User::get_by_username(&username)
                .await
                .map_err(server_error)?
            {
                Some(u) => {
                    let has_navidrome = MediaServerConfig::get(&u.id, MediaServerKind::Navidrome)
                        .await
                        .map_err(server_error)?
                        .is_some();
                    if u.media_account_id.is_some() && !has_navidrome {
                        return Err(unauthorized_error(
                            "This username is linked to a Plex account",
                        ));
                    }
                    u
                }
                None => User::create(&username, &password)
                    .await
                    .map_err(server_error)?,
            };

            let encrypted = crypto::encrypt(&password).map_err(server_error)?;
            let existing = MediaServerConfig::get(&user.id, MediaServerKind::Navidrome)
                .await
                .map_err(server_error)?;
            let url = existing
                .as_ref()
                .and_then(|c| c.url.clone())
                .or_else(|| stored_url.clone())
                .unwrap_or_default();
            MediaServerConfig::upsert_navidrome(
                &user.id,
                &url,
                false,
                &encrypted,
                &username,
                MediaServerStatus::Connected.as_str(),
            )
            .await
            .map_err(server_error)?;

            User::update_media(
                &user.id,
                "navidrome",
                Some(&encrypted),
                MediaServerStatus::Connected.as_str(),
                user.media_account_id.as_deref(),
            )
            .await
            .map_err(server_error)?;

            User::update_password(&user.id, &password)
                .await
                .map_err(server_error)?;

            let _ = models::user_settings::UserSettings::reset_navidrome_banner(&user.id).await;
            evict_media_server(&user.id).await;

            issue_session(
                user.id,
                user.username,
                MediaServerKind::Navidrome,
                MediaServerStatus::Connected,
                cookies,
            )
        }
        NavidromeAuthResult::AuthFailed => {
            let user = User::get_by_username(&username)
                .await
                .map_err(server_error)?
                .ok_or_else(|| unauthorized_error("Invalid username or password"))?;

            let _ = User::verify(&username, &password)
                .await
                .map_err(|_| unauthorized_error("Invalid username or password"))?;

            let navidrome = MediaServerConfig::get(&user.id, MediaServerKind::Navidrome)
                .await
                .map_err(server_error)?;
            let has_navidrome = navidrome.is_some();
            if user.media_account_id.is_some() && !has_navidrome {
                return Err(unauthorized_error(
                    "This account is linked to a different media server",
                ));
            }

            if has_navidrome {
                let _ = MediaServerConfig::update_status(
                    &user.id,
                    MediaServerKind::Navidrome,
                    MediaServerStatus::InvalidCredentials.as_str(),
                )
                .await;
            }

            User::update_media(
                &user.id,
                "navidrome",
                navidrome.as_ref().and_then(|c| c.token.as_deref()),
                MediaServerStatus::InvalidCredentials.as_str(),
                user.media_account_id.as_deref(),
            )
            .await
            .map_err(server_error)?;

            evict_media_server(&user.id).await;

            issue_session(
                user.id,
                user.username,
                MediaServerKind::Navidrome,
                MediaServerStatus::InvalidCredentials,
                cookies,
            )
        }
        result @ (NavidromeAuthResult::Unreachable | NavidromeAuthResult::NotConfigured) => {
            let status = match result {
                NavidromeAuthResult::NotConfigured => MediaServerStatus::Unknown,
                _ => MediaServerStatus::Offline,
            };
            let user = User::verify(&username, &password)
                .await
                .map_err(|_| unauthorized_error("Invalid username or password"))?;

            let navidrome = MediaServerConfig::get(&user.id, MediaServerKind::Navidrome)
                .await
                .map_err(server_error)?;
            let has_navidrome = navidrome.is_some();
            if user.media_account_id.is_some() && !has_navidrome {
                return Err(unauthorized_error(
                    "This account is linked to a different media server",
                ));
            }

            if has_navidrome {
                let _ = MediaServerConfig::update_status(
                    &user.id,
                    MediaServerKind::Navidrome,
                    status.as_str(),
                )
                .await;
            }

            // Keep the existing token so it works again once Navidrome is back.
            User::update_media(
                &user.id,
                "navidrome",
                navidrome.as_ref().and_then(|c| c.token.as_deref()),
                status.as_str(),
                user.media_account_id.as_deref(),
            )
            .await
            .map_err(server_error)?;

            issue_session(
                user.id,
                user.username,
                MediaServerKind::Navidrome,
                status,
                cookies,
            )
        }
    }
}

#[cfg(feature = "server")]
async fn login_plex(
    username: String,
    password: String,
    cookies: Cookies,
) -> Result<AuthResponse, ServerFnError> {
    use crate::models::media_server::MediaServerConfig;
    use models::user::User;
    use shared::system::MediaServerStatus;

    let user = User::verify(&username, &password)
        .await
        .map_err(|_| unauthorized_error("Invalid username or password"))?;

    let config = MediaServerConfig::get(&user.id, MediaServerKind::Plex)
        .await
        .map_err(server_error)?;

    let (kind, status) = match &config {
        Some(config) => {
            let status = MediaServerStatus::from(config.status.as_str());
            User::set_active_media(&user.id, "plex", config.token.as_deref(), status.as_str())
                .await
                .map_err(server_error)?;
            (MediaServerKind::Plex, status)
        }
        None => (
            MediaServerKind::from(user.media_server.as_str()),
            MediaServerStatus::from(user.media_status.as_str()),
        ),
    };

    issue_session(user.id, user.username, kind, status, cookies)
}

#[cfg(feature = "server")]
pub(crate) fn issue_session(
    user_id: String,
    username: String,
    media_server: MediaServerKind,
    media_status: shared::system::MediaServerStatus,
    cookies: Cookies,
) -> Result<AuthResponse, ServerFnError> {
    let token = auth::create_token(user_id.clone(), username.clone()).map_err(server_error)?;
    cookies.add(build_auth_cookie(token));

    Ok(AuthResponse {
        username,
        user_id,
        media_server,
        media_status,
    })
}

#[post("/api/auth/refresh", auth: AuthSession, cookies: Cookies)]
pub async fn refresh_token() -> Result<(), ServerFnError> {
    let claims = auth.0;

    let _ = models::user::User::get_by_id(&claims.sub)
        .await
        .map_err(server_error)?;

    let token = auth::create_token(claims.sub, claims.username).map_err(server_error)?;

    cookies.add(build_auth_cookie(token));

    Ok(())
}

#[post("/api/auth/logout", cookies: Cookies)]
pub async fn logout() -> Result<(), ServerFnError> {
    let mut cookie = Cookie::new(AUTH_COOKIE_NAME, "");
    cookie.set_path("/");

    cookies.remove(cookie);

    Ok(())
}

#[get("/api/auth/me", auth: AuthSession)]
pub async fn get_current_user() -> Result<Option<AuthResponse>, ServerFnError> {
    let claims = auth.0;

    let user = models::user::User::get_by_id(&claims.sub).await.ok();

    Ok(user.map(|u| AuthResponse {
        username: claims.username,
        user_id: claims.sub,
        media_server: MediaServerKind::from(u.media_server.as_str()),
        media_status: shared::system::MediaServerStatus::from(u.media_status),
    }))
}
