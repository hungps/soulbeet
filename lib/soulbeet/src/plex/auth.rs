use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Deserializer};
use shared::media_server::{PlexPin, PlexServerOption};
use url::Url;

use crate::error::{Result, SoulseekError};
use crate::http::build_client;

use super::client::PlexClientBuilder;

const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

pub const DEFAULT_PLEX_TV_URL: &str = "https://plex.tv";
const AUTH_APP_URL: &str = "https://app.plex.tv/auth";

#[derive(Debug, Clone)]
pub struct PlexTvUser {
    pub id: String,
    pub username: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct RawPin {
    #[serde(deserialize_with = "deserialize_id")]
    id: String,
    #[serde(default)]
    code: String,
    #[serde(default, rename = "authToken")]
    auth_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawUser {
    #[serde(deserialize_with = "deserialize_id")]
    id: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawResource {
    #[serde(default)]
    name: String,
    #[serde(default)]
    client_identifier: String,
    #[serde(default)]
    provides: String,
    #[serde(default)]
    connections: Vec<RawConnection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConnection {
    #[serde(default)]
    uri: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    local: bool,
    #[serde(default)]
    relay: bool,
}

fn deserialize_id<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!(
            "unexpected Plex id: {}",
            other
        ))),
    }
}

fn tv_url(base: &str, path: &str) -> Result<Url> {
    let mut normalized = base.trim_end_matches('/').to_string();
    normalized.push('/');
    Ok(Url::parse(&normalized)?.join(path)?)
}

fn client_request(client: &Client, method: reqwest::Method, url: Url, client_id: &str) -> reqwest::RequestBuilder {
    client
        .request(method, url)
        .header("X-Plex-Product", "Soulbeet")
        .header("X-Plex-Version", env!("CARGO_PKG_VERSION"))
        .header("X-Plex-Client-Identifier", client_id)
        .header("Accept", "application/json")
}

async fn send_json<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
    context: &str,
) -> Result<T> {
    let resp = request.send().await.map_err(|e| SoulseekError::Api {
        status: if e.is_timeout() { 408 } else { 503 },
        message: format!("Plex.tv {} failed: {}", context, e),
    })?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(SoulseekError::Api {
            status: status.as_u16(),
            message: format!("Plex.tv {} failed ({}): {}", context, status, body),
        });
    }
    serde_json::from_str(&body).map_err(|e| SoulseekError::Api {
        status: 500,
        message: format!("Failed to parse Plex.tv {} response: {}", context, e),
    })
}

/// Create a new plex.tv login PIN. The user signs in to `auth_url`, then the
/// app polls `check_pin` until plex.tv returns an account token.
pub async fn create_pin(plex_tv_url: &str, client_id: &str) -> Result<PlexPin> {
    let client: Client = build_client("Soulbeet");
    let url = tv_url(plex_tv_url, "api/v2/pins?strong=true")?;
    let request = client_request(&client, reqwest::Method::POST, url, client_id);
    let pin: RawPin = send_json(request, "create pin").await?;
    if pin.id.is_empty() || pin.code.is_empty() {
        return Err(SoulseekError::Api {
            status: 500,
            message: "Plex.tv did not return a login PIN".to_string(),
        });
    }

    let auth_url = format!(
        "{}#?clientID={}&code={}&context%5Bdevice%5D%5Bproduct%5D=Soulbeet",
        AUTH_APP_URL, client_id, pin.code
    );

    Ok(PlexPin {
        id: pin.id,
        code: pin.code,
        auth_url,
    })
}

/// Poll a PIN. Returns `None` while the user has not finished signing in.
pub async fn check_pin(
    plex_tv_url: &str,
    client_id: &str,
    pin_id: &str,
) -> Result<Option<PlexTvUser>> {
    let client: Client = build_client("Soulbeet");
    let url = tv_url(plex_tv_url, &format!("api/v2/pins/{}", pin_id))?;
    let request = client_request(&client, reqwest::Method::GET, url, client_id);
    let pin: RawPin = match send_json(request, "check pin").await {
        Ok(pin) => pin,
        Err(SoulseekError::Api { status: 404, .. }) => {
            return Err(SoulseekError::Api {
                status: 404,
                message: "Plex login request expired".to_string(),
            })
        }
        Err(e) => return Err(e),
    };

    let Some(token) = pin.auth_token.filter(|t| !t.is_empty()) else {
        return Ok(None);
    };

    let url = tv_url(plex_tv_url, "api/v2/user")?;
    let request = client_request(&client, reqwest::Method::GET, url, client_id)
        .header("X-Plex-Token", &token);
    let user: RawUser = send_json(request, "fetch user").await?;
    let username = if user.username.trim().is_empty() {
        user.title
    } else {
        user.username
    };

    Ok(Some(PlexTvUser {
        id: user.id,
        username,
        token,
    }))
}

/// List the Plex Media Servers the signed-in account can access. Each
/// connection is flattened into its own option so the user can choose LAN,
/// WAN or relay addresses.
pub async fn list_servers(
    plex_tv_url: &str,
    client_id: &str,
    token: &str,
) -> Result<Vec<PlexServerOption>> {
    let client: Client = build_client("Soulbeet");
    let url = tv_url(
        plex_tv_url,
        "api/v2/resources?includeHttps=1&includeRelay=1",
    )?;
    let request = client_request(&client, reqwest::Method::GET, url, client_id)
        .header("X-Plex-Token", token);
    let resources: Vec<RawResource> = send_json(request, "list servers").await?;

    let mut servers = Vec::new();
    for resource in resources {
        if !resource
            .provides
            .split(',')
            .any(|p| p.trim().eq_ignore_ascii_case("server"))
        {
            continue;
        }

        for connection in resource.connections {
            let (address, port) = connection_address(&connection);
            if address.is_empty() || port == 0 {
                continue;
            }
            servers.push(PlexServerOption {
                name: resource.name.clone(),
                machine_identifier: resource.client_identifier.clone(),
                address,
                port,
                secure: connection.protocol.eq_ignore_ascii_case("https"),
                local: connection.local,
                relay: connection.relay,
                uri: connection.uri.clone(),
            });
        }
    }

    Ok(servers)
}

fn connection_address(connection: &RawConnection) -> (String, u16) {
    if !connection.address.is_empty() && connection.port > 0 {
        return (connection.address.clone(), connection.port);
    }

    let Ok(uri) = Url::parse(&connection.uri) else {
        return (connection.address.clone(), connection.port);
    };
    let address = uri.host_str().unwrap_or(&connection.address).to_string();
    let port = uri.port_or_known_default().unwrap_or(connection.port);
    (address, port)
}

pub fn connection_url(server: &PlexServerOption) -> String {
    server.url(server.secure)
}

#[derive(Debug, Clone)]
pub struct ReachableServer {
    pub option: PlexServerOption,
    pub url: String,
    pub latency: Duration,
}

pub async fn select_reachable_server(
    client_id: &str,
    token: &str,
    servers: &[PlexServerOption],
) -> Option<ReachableServer> {
    let probes = servers.iter().cloned().map(|option| {
        let url = connection_url(&option);
        let client_id = client_id.to_string();
        let token = token.to_string();
        async move {
            let client = PlexClientBuilder::new()
                .base_url(&url)
                .token(&token)
                .client_id(&client_id)
                .build()
                .ok()?;
            let started = Instant::now();
            tokio::time::timeout(PROBE_TIMEOUT, client.ping())
                .await
                .ok()?
                .ok()?;
            let latency = started.elapsed();
            Some(ReachableServer {
                option,
                url,
                latency,
            })
        }
    });

    let mut reachable: Vec<ReachableServer> = futures::future::join_all(probes)
        .await
        .into_iter()
        .flatten()
        .collect();

    reachable.sort_by_key(|server| {
        (
            server.option.relay,
            !server.option.local,
            !server.option.secure,
            server.latency,
        )
    });
    reachable.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_connection_urls() {
        let server = PlexServerOption {
            name: "Home".to_string(),
            machine_identifier: "machine".to_string(),
            address: "192.168.1.10".to_string(),
            port: 32400,
            secure: false,
            local: true,
            relay: false,
            uri: String::new(),
        };
        assert_eq!(connection_url(&server), "http://192.168.1.10:32400");
    }

    #[test]
    fn prefers_the_advertised_uri() {
        let server = PlexServerOption {
            name: "Home".to_string(),
            machine_identifier: "machine".to_string(),
            address: "192.168.1.10".to_string(),
            port: 32400,
            secure: true,
            local: true,
            relay: false,
            uri: "https://192-168-1-10.machine.plex.direct:32400".to_string(),
        };
        assert_eq!(
            connection_url(&server),
            "https://192-168-1-10.machine.plex.direct:32400"
        );
        assert_eq!(server.url(false), "http://192.168.1.10:32400");
    }

    #[test]
    fn brackets_ipv6_addresses() {
        let server = PlexServerOption {
            name: "Home".to_string(),
            machine_identifier: "machine".to_string(),
            address: "2001:db8::1".to_string(),
            port: 32400,
            secure: true,
            local: true,
            relay: false,
            uri: String::new(),
        };
        assert_eq!(connection_url(&server), "https://[2001:db8::1]:32400");
    }

    #[test]
    fn accepts_numeric_and_string_ids() {
        let numeric = r#"{"id":123,"code":"abcd"}"#;
        let parsed: RawPin = serde_json::from_str(numeric).unwrap();
        assert_eq!(parsed.id, "123");

        let text = r#"{"id":"abc","code":"abcd"}"#;
        let parsed: RawPin = serde_json::from_str(text).unwrap();
        assert_eq!(parsed.id, "abc");
    }

    #[test]
    fn parses_resources_with_connections() {
        let json = r#"[{
            "name": "Home",
            "clientIdentifier": "machine",
            "provides": "server,client",
            "connections": [
                {"uri":"http://192.168.1.10:32400","address":"192.168.1.10","port":32400,"protocol":"http","local":true,"relay":false},
                {"uri":"https://plex.example.com:443","address":"plex.example.com","port":443,"protocol":"https","local":false,"relay":false}
            ]
        }]"#;
        let resources: Vec<RawResource> = serde_json::from_str(json).unwrap();
        assert_eq!(resources[0].connections.len(), 2);
        assert_eq!(
            connection_address(&resources[0].connections[1]),
            ("plex.example.com".to_string(), 443)
        );
    }

    #[test]
    fn falls_back_to_uri_when_address_is_missing() {
        let connection: RawConnection =
            serde_json::from_str(r#"{"uri":"http://10.0.0.2:32400","protocol":"http"}"#).unwrap();
        assert_eq!(
            connection_address(&connection),
            ("10.0.0.2".to_string(), 32400)
        );
    }
}
