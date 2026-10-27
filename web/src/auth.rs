use api::auth::AuthResponse;
use dioxus::prelude::*;
use ui::Auth;

pub fn use_auth() -> Auth {
    use_context::<Auth>()
}

#[component]
pub fn AuthProvider(children: Element) -> Element {
    let auth_state =
        use_resource(move || async move { api::get_current_user().await.ok().flatten() });
    // None until the session has been read. The router must not mount before
    // that: its first render can rewrite the url, and the auth guard would see
    // a logged-out user and send the load through /login to /.
    let loaded = auth_state.read().clone();

    let mut auth_signal = use_signal(|| None::<AuthResponse>);

    use_effect(move || {
        let user = auth_state.read().clone().flatten();
        auth_signal.set(user);
    });

    use_context_provider(|| Auth::new(auth_signal));

    if loaded.is_none() {
        return rsx! {
            div { class: "flex flex-col items-center justify-center h-screen",
                div { class: "animate-spin rounded-full h-16 w-16 border-t-4 border-b-4 border-beet-accent mb-6" }
                h1 { class: "text-2xl font-bold tracking-tighter uppercase text-transparent bg-clip-text bg-gradient-to-r from-white to-gray-400",
                    "SoulBeet"
                }
            }
        };
    }

    rsx! {
        {children}
    }
}
