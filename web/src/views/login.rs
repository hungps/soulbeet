use std::future::Future;
use std::pin::Pin;

use api::auth::AuthResponse;
use api::login;
use dioxus::prelude::*;
use shared::media_server::MediaServerKind;
use ui::Login;

use crate::auth::use_auth;
use crate::Route;

#[component]
pub fn LoginPage() -> Element {
    let navigator = use_navigator();
    let mut auth = use_auth();

    let login = use_callback(move |(username, password, server): (String, String, Option<MediaServerKind>)|
        -> Pin<Box<dyn Future<Output = Result<(), String>>>>
    {
        Box::pin(async move {
            match login(username, password, server).await {
                Ok(response) => {
                    auth.login(response);
                    navigator.push(Route::SearchPage {});
                    Ok(())
                }
                Err(ServerFnError::ServerError { message, .. }) => Err(message),
                Err(e) => Err(e.to_string()),
            }
        })
    });

    let on_plex_login = use_callback(move |response: AuthResponse| {
        auth.login(response);
        navigator.push(Route::SearchPage {});
    });

    let servers = use_resource(|| async { api::get_auth_servers().await.unwrap_or_default() });
    let loaded = servers.read().clone();

    match loaded {
        Some(servers) => rsx! {
            Login { login, servers, on_plex_login }
        },
        None => rsx! {
            div { class: "flex items-center justify-center min-h-screen text-gray-400 font-mono",
                "Loading..."
            }
        },
    }
}
