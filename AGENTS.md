# AGENTS.md

Guidance for agents working in this repository. Read this before making changes.

## What this project is

Soulbeet is a self-hosted music downloader, library manager, and discovery engine. One Rust
binary serves a Dioxus WASM SPA and a JSON/WebSocket server API, backed by SQLite. It orchestrates
four external systems:

- **slskd** (Soulseek) — source search and file downloads, v0.26+ (batches API).
- **beets** — tagging/import, invoked as the `beet` CLI subprocess.
- **Navidrome** — playback, ratings, and path-filtered discovery smart playlists.
- **Plex** — playback, ratings, and materialised discovery playlists.

A media server is optional; without a linked Navidrome or Plex account the app runs in
local-account mode with discovery/rating features off. Plex is linked through plex.tv OAuth;
Navidrome URL and credentials are configured per user in **Settings > Media Server**. Both may
be configured per user at once and the active one is selected in the same panel.

Version currently `0.7.1`. Default branch `master`.

## Workspace map

```
api/          Server functions, models, DB, auth, provider registry   `server` feature
ui/           Shared Dioxus components and contexts, no business logic
web/          Binary entrypoint: routes, auth guard, navbar, websocket
lib/shared/   Serde DTOs shared by client and server (compiles to WASM, keep dependency-free)
lib/soulbeet/ External integrations + recommendation engine (server-only)
desktop/, mobile/  Present but NOT workspace members (commented out in Cargo.toml)
```

Dependency direction: `web -> ui -> api -> soulbeet -> shared`. Never make `shared` depend on
anything server-side.

## Where things live

| Concern | Location |
|---|---|
| Env config singleton | `api/src/config.rs` (`CONFIG`) |
| SQLite pool + migrations | `api/src/db.rs` (pool max 5, `sqlx::migrate!("./migrations")`) |
| Provider/backend/importer registry | `api/src/services.rs` |
| Per-user broadcast channels, cancellation, discovery progress | `api/src/globals.rs` |
| JWT create/verify | `api/src/auth.rs` |
| AES-GCM credential encryption | `api/src/crypto.rs` |
| RPC endpoints | `api/src/server_fns/{auth,media_server,search,download,discovery,navidrome,settings,folder,user,system}.rs` |
| Row structs + queries | `api/src/models/` (raw `sqlx::query_as`, no ORM) |
| Media-server links + path mappings | `api/src/models/media_server.rs`, `api/migrations/*_media_server*.sql` |
| Integration traits (extension seams) | `lib/soulbeet/src/traits.rs` |
| slskd client / scoring | `lib/soulbeet/src/slskd/{client,models,processing,utils}.rs` |
| Media-server trait (extension seam) | `lib/soulbeet/src/traits.rs` (`MediaServer`, `PlaylistSpec`) |
| Navidrome client / Subsonic models | `lib/soulbeet/src/navidrome/{client,models}.rs` |
| Plex client / plex.tv OAuth | `lib/soulbeet/src/plex/{client,models,auth}.rs` |
| beets importer | `lib/soulbeet/src/beets/mod.rs` |
| Recommendation pipeline | `lib/soulbeet/src/engine/` |
| Routes and app shell | `web/src/main.rs`, `web/src/views/` |
| Reusable UI | `ui/src/components/` |
| Migrations | `api/migrations/*.sql` |

## Non-negotiable conventions

1. **Feature-gate every server-only item.** `api` and `ui` must still compile for WASM.
   Server code goes behind `#[cfg(feature = "server")]`; `ui`'s `server` feature forwards to
   `api/server`. Ungated code in `api` becomes client-visible.
2. **Authenticate every endpoint** with the `AuthSession` extractor. Map errors through
   `server_error` (500) or `unauthorized_error` (401) from `api/src/server_fns/mod.rs`.
3. **Never hold a Dioxus signal borrow across `.await`.** `clippy.toml` denies
   `await-holding-invalid-types` for `dioxus_signals::Write`, `GenerationalRef`,
   and `GenerationalRefMut`.
4. **Keep filesystem operations inside their root.** Use `cleanup_empty_ancestors(dir, root)`;
   it deliberately never climbs above `root` and skips paths outside it.
5. **No new dependencies in `lib/shared`** that do not compile to WASM.
6. **New DB columns require a new timestamped migration**; do not edit applied migrations.
7. **Access media servers through `MediaServer`** (`services::media_server_for_user`), never
   through `NavidromeClient`/`PlexClient` directly from endpoints. Ratings are normalised to
   1-5 stars and paths to `MediaTrack.path` by each implementation.
8. Follow local style: no comments in code you add, small focused changes, no opportunistic
   refactors.

## Core flows to understand before editing

**Search -> download -> import**
`search_album`/`search_track` -> metadata provider -> `start_download_search` (hydrates album
tracklists first) -> `poll_download_search` -> `download`/`auto_download` -> slskd batch enqueue
-> `DownloadMonitor` polls per-peer transfers, fails over (`download/failover.rs`), drops abandoned
transfers, broadcasts `DownloadEvent` over `/api/downloads/updates` -> `download/process.rs`
resolves the file on disk with retry/backoff -> `download/import.rs` -> beets -> target folder.
`auto_download` requires a source score >= `0.7`, otherwise returns results for manual choice.

**Media server selection**
`available_media_servers()` always exposes Navidrome and Plex. Plex login creates a PIN, opens
plex.tv, polls the PIN, fetches the account token and `/api/v2/resources`, then creates a local
user or links the current one. `media_server_configs` stores per-user URL, token, account and
server for each kind; `users.media_server` selects the active kind and
`users.media_token`/`media_status` mirror the active config for the session/navbar. Navidrome
login pings Subsonic with the supplied password using the URL saved for that local user; users
without a saved URL fall back to the local Argon2 password, so the first login on a fresh install
is always local, after which Navidrome is connected in Settings.
`media_path_mappings` translates local paths to server paths (and back) for ratings, discovery
playlists and Plex rescan requests. Verified with `Auth::refresh()` after settings changes.

**Discovery**
`start_discovery_generation` writes progress before spawning (avoids a poll race) ->
`generate_discovery_playlist_internal` runs pipeline per profile, downloads into
`<folder>/Discovery/<Profile>/`, records `discovery_tracks` -> `reconcile_discovery_playlists`
creates/updates Navidrome playlists -> `sync_ratings_internal` promotes 3+ stars and deletes
1-star tracks (average across users on shared folders).

**Automation** (`globals.rs::run_automation`) runs every 6 hours after a 30s startup delay:
ReportRealPath check -> rating sync -> playlist reconciliation -> recommendation generation ->
expiry of old tracks -> gap refill. It also calls `reset_stale_promoting` to recover from a crash.

**Auth**: login tries Navidrome `ping()` first when the local user has a saved Navidrome URL; on
success it stores the encrypted password. Otherwise (no saved URL, rejection, or unreachability)
it falls back to the local Argon2 hash. Navidrome status is persisted per user and drives the
navbar banner in `web/src/main.rs`.

## Build, run, test

```bash
./css.sh                                        # Tailwind v4 watcher -> web/assets/tailwind.css
dx serve --package web --platform web           # dev server (default port 9765)
cargo test -p api -p soulbeet                   # Rust unit tests (inline #[cfg(test)] modules)
cargo check -p web --features server            # server-side type check (WASM target not always installed)
cargo clippy                                    # respects clippy.toml await-borrow denies
```

E2E lives in `e2e/` (Playwright + Docker Compose with stub slskd/MusicBrainz and a real Navidrome):

```bash
cd e2e && npm ci && npm run lint && npm run typecheck
npm run fixtures && docker compose -f compose.e2e.yml up -d --build
npx playwright test
```

Native stack without Docker: `e2e/scripts/dev-stack.sh up|down`.

Production image: `Dockerfile`, tiers `light` (default), `medium`, `full` from
`build/tiers/*.env`; `:latest` aliases `:light`. The build runs `dx bundle --package web --release`
and the runtime serves `target/dx/web/release/web`.

## Environment variables

| Variable | Meaning | Default |
|---|---|---|
| `DATABASE_URL` | SQLite connection string | `sqlite:soulbeet.db` |
| `SECRET_KEY` | JWT signing + credential encryption | insecure `secret` (warns) |
| `BEETS_CONFIG` | beets config path | `beets_config.yaml` |
| `PORT`, `IP` | HTTP bind | `9765`, `0.0.0.0` |

slskd URL/API key, download paths, album mode and alternative MusicBrainz servers live in the
DB (`app_config`, key constants in `api/src/models/app_config.rs`) and are set through the UI,
not env. Scrobble credentials are
per-user in `user_settings`.

## Gotchas

- State is process-local: `USER_CHANNELS`, `DISCOVERY_PROGRESS`, and provider caches are
  statics. Multi-replica deployment breaks WebSocket fan-out and discovery progress.
- Each music folder owns `<folder>/.beets_library.db`; duplicates are only detectable per folder,
  never across libraries.
- Navidrome discovery playlists are smart playlists filtered by path and self-update. Plex has no
  documented path-filtered smart-playlist API, so `PlexClient::reconcile_playlist` materialises a
  regular playlist from the pending `discovery_tracks` rows and the server's track list; new files
  are only picked up after Plex indexes them (best-effort `/activities` polling).
- Plex ratings are per plex.tv account and the API exposes no cross-user average, so the Navidrome
  shared-folder veto does not apply. Ratings are mapped 0-10 -> 1-5 stars (`div_ceil(2).min(5)`).
- Plex is linked through plex.tv OAuth, not a username/password. Accounts created by the OAuth
  login get a random local password; the Plex panel's **Logout** unlinks the account without
  ending the Soulbeet session. Linking an existing local account is done from Settings > Media
  Server and requires an authenticated session.
- One user may keep both Navidrome and Plex configs. `users.media_account_id` identifies the Plex
  account; Navidrome login rejects a username that is Plex-linked but has no Navidrome config.
- Path mappings (`media_path_mappings`) take precedence after direct/relative path checks and are
  the only way to translate paths between containers. Downloads trigger a Plex section refresh
  for the mapped target path; Navidrome falls back to a full scan.
- Plex OAuth uses a PIN opened in a popup. The UI opens `about:blank` synchronously to avoid
  popup blockers, then points it at `app.plex.tv`. Polling runs for roughly five minutes.
- Restarting the server loses in-memory discovery progress (terminal states are pruned after
  5 minutes when polled, 30 minutes by cleanup).
- WebSocket broadcast channel capacity is 100; lag is logged and skipped, not fatal.
- `desktop/` and `mobile/` crates are not built or tested by the workspace — treat changes there
  as unverified.
- slskd transfers can complete before the file is visible through the shared volume; path
  resolution retries with exponential backoff for roughly 15s.
- The beets image is quiet (`-q`), so ambiguous imports are skipped unless `beets_config.yaml`
  is made permissive (see README "Album Mode").

## Adding things

- **New endpoint**: add to the relevant `api/src/server_fns/*.rs`; `server_fns/mod.rs` re-exports
  with `pub use`.
- **New download/metadata backend**: implement the trait in `lib/soulbeet/src/traits.rs` (or a new
  module), register it in `api/src/services.rs`, expose it in the `available_*` lists.
- **New UI feature**: page shell in `web/src/views/`, reusable pieces in `ui/src/components/`.
- **New persisted entity**: migration in `api/migrations/`, struct + queries in `api/src/models/`,
  DTO for the client in `lib/shared/` if it crosses the wire.
- **Recommendation tuning**: `lib/soulbeet/src/engine/{profile,blender,freshness,diversifier}.rs`
  and `ProfileConfig` in `lib/shared/src/recommendation.rs`.
