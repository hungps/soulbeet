//! Exact quality numbers from the beets database.
//!
//! `beet ls -f '$bitrate $samplerate'` prints the fields the way beets shows
//! them, `128kbps` and `44kHz`, and that kHz is truncated: 44100 Hz reads as
//! `44`. The values beets stores in its own SQLite file are the raw ones, so a
//! scan reads them back and fills in what the listing lost.

use std::collections::HashMap;
use std::path::Path;

use dioxus::logger::tracing::debug;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

use soulbeet::library::ScannedLibrary;

/// What the beets database says about one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Quality {
    /// kbps, the unit the library stores.
    bitrate: Option<u32>,
    bit_depth: Option<u32>,
    sample_rate: Option<u32>,
}

fn kbps(bits_per_second: Option<i64>) -> Option<u32> {
    bits_per_second
        .filter(|value| *value > 0)
        .map(|value| (value as f64 / 1000.0).round() as u32)
}

fn positive(value: Option<i64>) -> Option<u32> {
    value
        .filter(|value| *value > 0)
        .map(|value| value as u32)
}

/// Patches `scan` with the values beets stored, matching its files by their
/// path relative to the library root. Silent when the database is missing or
/// unreadable: the scan's own values still stand.
pub async fn apply(root: &Path, scan: &mut ScannedLibrary) {
    let Some(quality) = read(root).await else {
        return;
    };
    let mut patched = 0usize;
    for release in &mut scan.releases {
        for track in &mut release.tracks {
            let Some(relative) = Path::new(&track.path)
                .strip_prefix(root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
            else {
                continue;
            };
            let Some(found) = quality.get(&relative) else {
                continue;
            };
            track.bitrate = found.bitrate.or(track.bitrate);
            track.bit_depth = found.bit_depth.or(track.bit_depth);
            track.sample_rate = found.sample_rate.or(track.sample_rate);
            patched += 1;
        }
    }
    debug!("Filled exact quality for {patched} tracks from the beets database");
}

async fn read(root: &Path) -> Option<HashMap<String, Quality>> {
    let database = root.join(".beets_library.db");
    if !database.is_file() {
        return None;
    }
    let options = SqliteConnectOptions::new()
        .filename(&database)
        .read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .ok()?;
    let rows = sqlx::query("SELECT path, bitrate, bitdepth, samplerate FROM items")
        .fetch_all(&pool)
        .await;
    pool.close().await;
    let rows = rows.ok()?;

    let mut quality = HashMap::new();
    for row in rows {
        let path: Vec<u8> = row.try_get("path").ok()?;
        let quality_row = Quality {
            bitrate: kbps(row.try_get("bitrate").ok().flatten()),
            bit_depth: positive(row.try_get("bitdepth").ok().flatten()),
            sample_rate: positive(row.try_get("samplerate").ok().flatten()),
        };
        if quality_row.bitrate.is_none()
            && quality_row.bit_depth.is_none()
            && quality_row.sample_rate.is_none()
        {
            continue;
        }
        quality.insert(String::from_utf8_lossy(&path).into_owned(), quality_row);
    }
    Some(quality)
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use soulbeet::library::{ScannedRelease, ScannedTrack};

    async fn beets_db(root: &Path, items: &[(&str, Option<i64>, Option<i64>, Option<i64>)]) {
        let options = SqliteConnectOptions::new()
            .filename(root.join(".beets_library.db"))
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE items (path BLOB, bitrate INTEGER, bitdepth INTEGER, samplerate INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        for (path, bitrate, bitdepth, samplerate) in items {
            sqlx::query("INSERT INTO items (path, bitrate, bitdepth, samplerate) VALUES (?, ?, ?, ?)")
                .bind(path.as_bytes().to_vec())
                .bind(bitrate)
                .bind(bitdepth)
                .bind(samplerate)
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
    }

    fn scan_of(root: &Path, name: &str) -> ScannedLibrary {
        ScannedLibrary {
            releases: vec![ScannedRelease {
                tracks: vec![ScannedTrack {
                    path: root.join(name).to_string_lossy().into_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            warnings: Vec::new(),
        }
    }

    #[tokio::test]
    async fn raw_values_replace_the_truncated_ones() {
        let tmp = tempfile::tempdir().unwrap();
        beets_db(
            tmp.path(),
            &[("Artist/Album/01.flac", Some(1411200), Some(24), Some(96000))],
        )
        .await;
        let mut scan = scan_of(tmp.path(), "Artist/Album/01.flac");

        apply(tmp.path(), &mut scan).await;

        let track = &scan.releases[0].tracks[0];
        assert_eq!(track.bitrate, Some(1411));
        assert_eq!(track.bit_depth, Some(24));
        assert_eq!(track.sample_rate, Some(96000));
    }

    #[tokio::test]
    async fn files_beets_does_not_know_keep_the_scanned_values() {
        let tmp = tempfile::tempdir().unwrap();
        beets_db(tmp.path(), &[("Artist/Album/02.flac", Some(1), Some(1), Some(1))]).await;
        let mut scan = scan_of(tmp.path(), "Artist/Album/01.flac");
        scan.releases[0].tracks[0].sample_rate = Some(44000);

        apply(tmp.path(), &mut scan).await;

        let track = &scan.releases[0].tracks[0];
        assert_eq!(track.sample_rate, Some(44000));
        assert_eq!(track.bit_depth, None);
    }

    #[tokio::test]
    async fn a_missing_database_leaves_the_scan_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let mut scan = scan_of(tmp.path(), "Artist/Album/01.flac");
        scan.releases[0].tracks[0].bitrate = Some(128);

        apply(tmp.path(), &mut scan).await;

        assert_eq!(scan.releases[0].tracks[0].bitrate, Some(128));
    }
}
