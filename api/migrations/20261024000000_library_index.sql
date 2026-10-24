CREATE TABLE IF NOT EXISTS library_releases (
    id TEXT PRIMARY KEY NOT NULL,
    folder_id TEXT NOT NULL,
    folder_path TEXT NOT NULL,
    album TEXT NOT NULL,
    album_artist TEXT NOT NULL,
    album_key TEXT NOT NULL,
    album_artist_key TEXT NOT NULL,
    year INTEGER,
    mb_album_id TEXT,
    release_path TEXT NOT NULL,
    track_count INTEGER NOT NULL DEFAULT 0,
    matched_track_count INTEGER NOT NULL DEFAULT 0,
    expected_track_count INTEGER,
    total_size INTEGER NOT NULL DEFAULT 0,
    scanned_at TEXT NOT NULL,
    FOREIGN KEY (folder_id) REFERENCES folders(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_library_releases_folder_id ON library_releases(folder_id);
CREATE INDEX IF NOT EXISTS idx_library_releases_album_keys
    ON library_releases(album_artist_key, album_key);
CREATE INDEX IF NOT EXISTS idx_library_releases_mbid ON library_releases(mb_album_id);

CREATE TABLE IF NOT EXISTS library_tracks (
    id TEXT PRIMARY KEY NOT NULL,
    release_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    path TEXT NOT NULL,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    album_artist TEXT NOT NULL,
    title_key TEXT NOT NULL,
    artist_key TEXT NOT NULL,
    album_key TEXT NOT NULL,
    track_number INTEGER,
    disc_number INTEGER,
    duration INTEGER,
    mb_track_id TEXT,
    mb_album_id TEXT,
    size INTEGER,
    matched INTEGER NOT NULL DEFAULT 0,
    indexed_at TEXT NOT NULL,
    FOREIGN KEY (release_id) REFERENCES library_releases(id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id) REFERENCES folders(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_library_tracks_release_id ON library_tracks(release_id);
CREATE INDEX IF NOT EXISTS idx_library_tracks_folder_id ON library_tracks(folder_id);
CREATE INDEX IF NOT EXISTS idx_library_tracks_path ON library_tracks(folder_id, path);
CREATE INDEX IF NOT EXISTS idx_library_tracks_title_keys ON library_tracks(artist_key, title_key);

CREATE TABLE IF NOT EXISTS library_scan_state (
    folder_id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL DEFAULT 'idle',
    last_scanned_at TEXT,
    error TEXT,
    release_count INTEGER NOT NULL DEFAULT 0,
    track_count INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (folder_id) REFERENCES folders(id) ON DELETE CASCADE
);
