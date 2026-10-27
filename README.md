# Soulbeet

> THIS FORK IS MADE WITH CLANKER, USE WITH CAUTION

> What's added:
> - Plex integration
> - Selfhosted MusicBrainz support
> - Library management, unknown release/track matching
> - Filter album by release type and medium
> - Able to pick specific album release
> - Download queue
> - Play downloaded track
> - Play track preview thanks to [Deezer](https://www.deezer.com/)

[![Docker Pulls](https://img.shields.io/docker/pulls/hungps/soulbeet)](https://hub.docker.com/r/hungps/soulbeet)
[![Docker Image Size](https://img.shields.io/docker/image-size/hungps/soulbeet)](https://hub.docker.com/r/hungps/soulbeet/tags)
[![Docker Image Version](https://img.shields.io/docker/v/hungps/soulbeet)](https://hub.docker.com/r/hungps/soulbeet/tags)

[![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/hungps/soulbeet/e2e.yml?branch=develop&label=e2e)](https://github.com/hungps/soulbeet/actions)
[![GitHub License](https://img.shields.io/github/license/hungps/soulbeet)](https://github.com/hungps/soulbeet)
[![GitHub Repo stars](https://img.shields.io/github/stars/hungps/soulbeet)](https://github.com/hungps/soulbeet)

Soulbeet is a self-hosted music downloader, library manager, and discovery engine. Search for music, download it, and let the app handle everything else: finding the best source, tagging, organizing, and keeping your library clean. Connect your scrobble history and Soulbeet will find new music for you automatically.

Screenshots: [here](./screenshots)

## Features

- **Search & Download**: Find albums and tracks, hit download. Soulbeet picks the best available source from Soulseek, downloads it, tags it with beets, and puts it in your library. No manual file management.
- **Music Discovery**: Soulbeet analyzes your Last.fm and ListenBrainz history, finds new music through track similarity, artist exploration, collaborative filtering, and genre discovery, downloads the best candidates, and pushes playlists to your Navidrome or Plex server. Fully automatic.
- **Three Discovery Profiles**: Conservative (close to what you know), Balanced, or Adventurous (unfamiliar territory). Run one or all three, each with its own playlist.
- **Rate & Keep**: Listen in Navidrome or Plex. 3+ stars promotes a track to your permanent library, 1 star deletes it. Unrated tracks expire and get replaced with fresh picks.
- **Multi-user**: Private or shared folders. Each user gets their own discovery profiles, scrobble credentials, and preferences.
- **Multiple Metadata Providers**: MusicBrainz, any alternative MusicBrainz-compatible servers configured in Settings > Config (BrainzMash is shipped as a preset), or Last.fm (single tracks), selectable from the search page. MusicBrainz searches expose release type and medium filters.

## How It Works

You search, you click download. Behind the scenes:

1. **Soulbeet** finds the best source on Soulseek, downloads it through **slskd**, tags and organizes it with **beets**, and puts it in your library.
2. **Navidrome** or **Plex** picks up the new files and makes them streamable.
3. For discovery: **Last.fm / ListenBrainz** feed your listening history into the recommendation engine, which finds new tracks, downloads them, and pushes playlists to your media server.

## Download Requests

Every download becomes a request, shown in the **Downloads** drawer (the icon in the navbar).

- One card per download. A card groups that download's files and shows how many have landed, plus a status: PENDING, QUEUED, DOWNLOADING, DOWNLOADED, TAGGING, TAGGED, SKIPPED, STOPPED or FAILED.
- Expand a card (click it) for its per-file rows. Each row has its own badge — QUEUED, SLSK, DOWNLOADED, WAIT, BEETS, LIB, SKIP, ERR, CANCEL or STOP — the file name, the audio quality and size, the speed and progress while it runs, and "Show beets output" for imports. A kept row shows the folder its files were left in and the reason it was kept.
- Cards keep the release selected in search (artist, country, medium, track count and date).
- Press play on a file row that is already on disk to preview it. The mini player at the bottom of the page has play/pause, stop and seek, and only one track plays at a time.
- **Stop request** cancels the running transfers and keeps the rows; **Resume request** queues them again.
- **Retry failed tracks** re-enqueues only the files that failed.
- **Remove request** cancels what is still running, deletes that request's downloaded files from the download folder and drops its rows. Files already imported into a library stay there.
- **Clear completed** / **Clear failed** dismiss finished cards. Clearing a row the server kept an import job for also drops that job, so its Retry import / match picker goes away.
- Downloading a release that is still in flight does not enqueue a second copy: a manual download attaches to the open card and skips files it already queued, and an auto download drops its duplicate card so the running batch takes the files.

When a search finds no source for a request, or nothing can be queued, the request is parked instead of failed: the search row shows **Requested** and the drawer shows **REQUESTED — Waiting to retry**. Soulbeet re-checks Soulseek for parked requests on the interval set in **Settings > Library > Retry unavailable requests** (off, 2h, 6h, 12h, 1 day or 1 week) and drops the parked request once a source is queued.

A Soulseek search starts with the artist and the album title. When that returns nothing, Soulbeet searches the album title on its own — Soulseek quietly drops some words, artist names included, from what it matches — and keeps only the results whose file paths still name the artist. Release qualifiers such as `(Deluxe Edition)` and `Original Soundtrack` are dropped next, and a single-track release is also searched by its track title. **Download Options** shows **Search again** when a search comes back empty, which runs the same search for that release once more.

## Self-Hosting with Docker

The recommended way to run Soulbeet is via Docker Compose. This ensures all dependencies (like `beets` and `python`) are correctly set up.

**Compatibility:** The Docker image supports both **AMD64** and **ARM64** architectures.

### Prerequisites

-   Docker & Docker Compose (or podman-compose)

### Quick Start

1.  Create a `docker-compose.yml` file:

```yaml
services:
  soulbeet:
    image: docker.io/hungps/soulbeet:latest
    restart: unless-stopped
    ports:
      - 9765:9765
    environment:
      - DATABASE_URL=sqlite:/data/soulbeet.db
      - SECRET_KEY=change-me-in-production
      # Optional
      - BEETS_CONFIG=/config/config.yaml
    volumes:
      - ./data:/data
      - ./beets:/config
      - /path/to/slskd/downloads:/downloads
      - /path/to/music:/music
```

slskd and your media server are not part of this snippet; point Soulbeet at them from the UI (see below).

2.  **Important**: The `/downloads` volume must point at the same physical folder `slskd` writes to, so Soulbeet can see the files it downloaded. If the two containers mount it under different paths, set **slskd downloads path** and **Soulbeet downloads path** in **Settings > Config** and Soulbeet translates the paths slskd reports. The `/music` volume must point at the same folder your media server (`navidrome` or `plex`) indexes, so it can see the organized library; use the **path mappings** in **Settings > Media Server** when the two sides differ. Soulbeet requires **slskd 0.26 or newer** (downloads are enqueued through the batches API introduced in 0.26).

3.  Seed the beets config. `BEETS_CONFIG` replaces the config shipped in the image, so start from a copy of it. Use the same tag as the `image:` line above, since the shipped `plugins:` line depends on the tier (see [Image Tiers](#image-tiers)), and copy it again after you change tier:

```bash
TAG=latest # the tag on your image: line (latest, light, medium or full)
mkdir -p beets
docker create --name sb "docker.io/hungps/soulbeet:$TAG"
docker cp sb:/app/beets_config.yaml ./beets/config.yaml
docker rm sb
```

4.  Run:

```bash
docker compose up -d
```

### Initial Setup

1.  Open `http://localhost:9765` and log in with the default `admin` / `admin` account (change it in **Settings > Users**). Accounts are local; a media server is linked per user in the next steps.
2.  In **Settings > Config**, connect slskd (URL + API key). [How to get an slskd API key](https://github.com/slskd/slskd/blob/master/docs/config.md#yaml-24).
3.  In **Settings > Library**, add your music folders (e.g. `/music`).
4.  In **Settings > Media Server**, choose the active server. Navidrome users can override the URL and credentials; Plex users can log in, pick a server from their plex.tv account, and toggle HTTPS. Add path mappings when Soulbeet and the media server mount the library at different paths.
5.  That's it. Search for something and download it.

For discovery (optional): add your Last.fm API key and/or ListenBrainz token in Settings > Library, enable discovery on a folder, pick your profiles, and hit Generate.

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | Connection string for SQLite | `sqlite:soulbeet.db` |
| `SECRET_KEY` | Encryption key for tokens and credentials | |
| `BEETS_CONFIG` | Path to your beets config file. Replaces the shipped `/app/beets_config.yaml` (beets defaults still apply underneath) | `beets_config.yaml` |
| `BEETS_PYTHON` | Python interpreter that runs beets imports and the config check. By default it is read from the `#!` line of `beet` (`/usr/bin/python3` in the image) | |
| `RUST_LOG` | Log filter. Use `info,soulbeet::beets::output=debug` to log every line beets prints. A bare `soulbeet::beets::output=debug` turns off all other info logs | |


**Note**: slskd URL, API key, download paths, album mode and alternative MusicBrainz servers are configured through the web UI (Settings > Config) and stored in the database. Scrobble credentials (Last.fm API key, ListenBrainz token) are configured per-user in Settings > Library. Navidrome and Plex URLs and credentials are configured per-user in Settings > Media Server.

**Note**: Alternative MusicBrainz servers are configured in Settings > Config. The shipped BrainzMash entry (`api.brainzmash.cc`) is a community pool of MusicBrainz mirrors; the public endpoint is gated by the pool operators. Replace it with an approved endpoint or your own MusicBrainz mirror if searches fail with HTTP 403.

### Beets Configuration

Soulbeet uses `beets` to import music. The image ships a config at `/app/beets_config.yaml`, with the `plugins:` line set for your image tier. Setting `BEETS_CONFIG` to another file **replaces** it, it does not merge with it. Copy the shipped file (see Quick Start) and edit the copy.

-   Which plugins each tier installs is listed in [docs/beets-plugins.md](./docs/beets-plugins.md). A plugin in `plugins:` that the tier does not install shows as a warning in Settings.
-   `plugins:` must contain `musicbrainz`. Without a metadata source plugin, Soulbeet marks beets as broken.
-   `include:` is ignored in the file you point `BEETS_CONFIG` at.
-   `mbtwopass` lives in `/opt/beets-plugins`. To keep it, set `pluginpath: [/opt/beets-plugins, /data/beets-plugins]`.
-   `discogs` needs `discogs.user_token`.
-   Soulbeet passes `--log`, which overrides `import.log`, and `--noincremental` and `--noresume`, which override `import.incremental` and `import.resume`.

Soulbeet runs every import through a small driver, `lib/soulbeet/src/beets/import.py`, with the Python that runs beets. The driver runs beets' own `beet` command line with the flags below and notes the matches beets weighed, so a skipped import can be finished with one of them (see [Skipped Imports](#skipped-imports)). It supports beets 2.8 to 2.11; with a beets it cannot hook into, Soulbeet marks beets as broken.

Default `beet import` flags used:
-   `-v`: Verbose output. The last lines of it (20 lines, 2 KiB) are shown under "beets output" on rows that carry a note: skipped, failed, or imported with a warning
-   `-c [config]`: The config file
-   `-q`: Quiet mode (no user interaction)
-   `--noincremental`: Import a folder again even if beets saw it before, so "Retry import" always runs
-   `--noresume`: Ignore beets' saved progress from an interrupted run
-   `-s`: Singleton mode (default behaviour unless Album mode is enabled in Settings > Config)
-   `-l [library_path]`: Library database path (per-folder)
-   `-d [target_path]`: Import to the specific folder selected in the web UI.
-   `--log [file]`: Where beets writes its import log

#### Skipped Imports

In quiet mode beets skips files it cannot match closely, and the row says "No close match". Soulbeet also skips an album match it would otherwise apply unseen when the folder leaves tracks of the release out, or when the match is not a strong recommendation (`match.strong_rec_thresh`). Soulbeet keeps the downloaded files and the row offers:
-   **Retry import**: the same import again, for example after you changed the beets config.

When beets handled the import as one task (one track, or one album folder that beets reads as one album), the row also offers the choices below. Show matches and Import as-is need the matches beets weighed for it; Force import needs only the selection stored with the request.
-   **Show matches**: the matches beets weighed for that import, with their score and what differs. Pick one, or paste a MusicBrainz link or ID (a recording for singletons, a release in album mode), then click **Import with this match**. beets applies exactly that match (`-S <id>`), however low its score.
-   **Import as-is**: beets imports the files with their own tags (`-A`) and looks nothing up. Every file needs artist and title tags, and in album mode an album tag too. If one lacks them, nothing is imported and the row names the files that lack them: beets would file untagged music under empty names, and with `duplicate_action: remove` a second untagged import would replace the first.
-   **Force import**: opens the same release-and-track dialog as **Dashboard > Library > TAG**, with the release selected when the download was requested already picked. Map each downloaded file onto a track, then Soulbeet runs one ordinary album import of the download folder with that mapping applied to each file by an import hook (`beet import -a -A`), so the files land under the normal album layout, with an album in the beets library, duplicate handling, and the artist, album, track and MusicBrainz ids you picked — beets just never does its own matching. Files left on **— skip —** stay in the download folder. It does not need beets' match list, so it also works when beets found no candidates at all. When a request predates the stored selection, Soulbeet recovers the MusicBrainz ID from its request key.

The reason, the beets output, the kept folder and whether the row can still be retried are stored with the request, and so is the import job itself (its files, target folder, album-or-single mode and the matches beets found). A page reload and a server restart therefore both keep **Retry import**, **Import as-is**, **Force import** and **Show matches** working. A job is dropped when you clear the row, retry it, force-import it without failures, remove its request, or when the newest 200 per user push it out.

### Image Tiers

Soulbeet ships three image tiers: `:light` (default, MusicBrainz tagging only), `:medium` (adds AcoustID fingerprint matching via the `chroma` plugin), and `:full` (adds cover art, lyrics, ReplayGain, and more). `:latest` is an alias for `:light` so existing installs keep their current behavior. See [docs/beets-plugins.md](./docs/beets-plugins.md) for what each tier ships, how to switch, and how to drop in your own plugins.

### Library Management

**Important**: Each music folder you configure in Soulbeet has its own beets database (`.beets_library.db`) stored at the root of that folder. This enables:

- Per-folder duplicate detection
- Independent library management for each user/folder

#### Interacting with Your Library

Each user can have multiple libraries. Each library is a folder that contains music files and a `.beets_library.db` file. This database is used to avoid duplicate tracks within the same library.

Since we use different databases, we can't directly compare tracks across libraries. However, we can use the `beets` CLI to interact with each library individually. This way you can add tracks outside of Soulbeet but keep them in sync with your library.

To manually interact with a library (list tracks, modify tags, remove items, etc.), use the `beet` CLI with the `-l` flag pointing to the folder's database:

```bash
# List all tracks in a library
beet -l /music/Person1/.beets_library.db ls
```

If using Docker, run these commands inside the container:
```bash
docker exec -it <container_name> beet -l /music/Person1/.beets_library.db ls
```

For more beets commands, see the [beets documentation](https://beets.readthedocs.io/en/stable/reference/cli.html).

#### Library Indexing

**Settings > Library** keeps a per-folder index that the Dashboard and the search results use (releases, track completeness, and whether a result is already in your library):

- **Periodic scan**: re-index a folder and synchronise its beets database on a schedule (off, 2h, 6h, 12h, 1 day or 1 week). Every folder is also scanned once at startup.
- **Native file watch**: re-index a folder as files change on disk, while the app is running.
- **Retry unavailable requests**: how often parked download requests are re-checked on Soulseek (see [Download Requests](#download-requests)).

Releases and tracks in **Dashboard > Library** show the audio quality read during the last scan, such as `FLAC 16/44.1` or `MP3 320k`, the same badge the search and transfer panels use. A release whose tracks use different formats is marked `Mixed`. The format comes from the file extension; the bitrate, bit depth and sample rate are read straight from the beets library database, because `beet ls` prints them rounded (`128kbps`, `44kHz`). Scan a folder again after upgrading if its rows predate these fields. Hover a track row and press play to preview the file in the mini player.

#### Album Mode

Enable **Album mode** in Settings > Config. Soulbeet then saves every download batch in its own folder named `<label> [<id>]` under the download path and imports that folder as an album instead of importing its files as singletons.

This option is only needed when you want album tags on your tracks. E.g: `albumartist` `mb_albumid` and so on..

**Important for Album Mode:**
Since `beets` is run in quiet mode (`-q`), it will skip any imports that require user intervention (e.g., if the metadata doesn't match confidently). A skipped album that beets read as one album can still be imported with a match you pick or as-is (see [Skipped Imports](#skipped-imports)). A folder that leaves tracks of the release out, or matches it too weakly, is always skipped to that prompt, whatever the config says.

Recommended additions to your config file for Album Mode:

```yaml
import:
  # quiet_fallback: asis # Optional: If no strong match, import with existing tags instead of skipping
  timid: no

match:
  strong_rec_thresh: 0.10  # Lower threshold to accept less confident matches
  max_rec:
    unmatched_tracks: strong
  distance_weights:
    # unmatched_tracks: 0.2  # Optional: Penalize extra files less.
```

*Note: Tweaking `strong_rec_thresh` and other matching parameters increases the risk of incorrect tags, but is necessary for fully automated imports of obscure releases.*

### Discovery Setup

Discovery generates personalized playlists from your scrobble history and pushes them to Navidrome or Plex. Here's how to set it up.

#### Media Server Configuration

Your Soulbeet folder paths must point to the same physical directories that the media server's music library uses. The mount paths inside each container can differ (e.g. Soulbeet at `/music`, Navidrome at `/media/music`), as long as they map to the same files on the host. Add a **path mapping** in **Settings > Media Server** (local path → server path) when the paths differ.

**Navidrome**

1. **Enable ReportRealPath** for the Soulbeet player. Go to your Navidrome instance > Settings > Players (e.g. `https://your-navidrome/app/#/player`), find the Soulbeet player entry, and enable "Report Real Path". Without this, rating sync and auto-delete cannot resolve file paths.

**Plex**

1. No player setting is required: Plex reports the indexed file path directly. Make sure the Plex token belongs to an account that can access the music library.

#### Soulbeet Configuration

1. Go to **Settings > Library** in the Soulbeet web UI
2. Add your **Last.fm API key** and/or **ListenBrainz username + token**
3. **Enable Discovery** on a folder and pick your profiles (Conservative, Balanced, Adventurous)
4. Optionally customize playlist names for each profile
5. Hit **Generate** to run the first batch

#### How It Works

- Discovery creates a `Discovery/` directory inside your chosen folder, with subdirectories per profile (e.g., `Discovery/Balanced/`, `Discovery/Adventurous/`).
- Tracks are downloaded from Soulseek, imported via beets into the profile subdirectory, and tagged properly.
- On Navidrome, a smart playlist is created (via the native API) for each profile, filtered by the folder path. The playlist auto-updates as Navidrome scans new files.
- On Plex, a regular playlist is created from the tracks currently in the profile's `Discovery/` folder and replaced on every reconciliation (6-hour automation, and after each generation once Plex finishes scanning).
- Every 6 hours (after a 30 s startup delay) the automation rechecks Navidrome's ReportRealPath, syncs ratings, recreates missing playlists, regenerates recommendations, expires old discovery tracks and refills the gaps. It does nothing while no media server is linked.

#### Auto-Delete

When enabled (Settings > Library > Auto-delete), 1-star tracks are deleted from disk during rating sync. This requires ReportRealPath on Navidrome so Soulbeet receives the actual file path; Plex reports paths directly. For shared folders on Navidrome, a track is only deleted if the average rating across all users is 1 or below. Plex ratings are per account, so that shared-veto check is unavailable.

## Development

1.  Install Rust and `dioxus_cli`.
2.  Run the tailwind watcher:
    ```bash
    ./css.sh
    ```
3.  Run the app:
    ```bash
    dx serve --package web --platform web
    ```
4.  Local imports need beets **2.8 to 2.11** on `PATH`; the image pins `beets==2.11.0` (see `build/requirements-beets.txt`). A newer beets is reported as **Broken** in Settings > Config, because the import driver hooks into beets internals. `BEETS_PYTHON` overrides the interpreter read from the `beet` shebang.
5.  Run the tests (the Python tests need Python 3.11+ with beets and pytest installed):
    ```bash
    cargo test --workspace --features server,dioxus/server --lib
    cargo test -p soulbeet --lib
    python -m pytest -q lib/soulbeet/tests/probe lib/soulbeet/tests/import
    ```

## Roadmap

- Reduce friction: fewer clicks between "I want this" and "it's in my library"
- Play preview on album track list before downloading
- Smarter Soulseek search (better source ranking, automatic fallback between providers)
