-- Manual and automatic downloads grouped into request sessions. A request is
-- one release; further downloads for the same release attach to the open
-- request instead of creating a new one.
CREATE TABLE IF NOT EXISTS download_requests (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    request_key TEXT NOT NULL,
    label TEXT NOT NULL,
    artist TEXT,
    target_folder TEXT NOT NULL,
    backend TEXT,
    state TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_download_requests_user_state
    ON download_requests (user_id, state);

CREATE TABLE IF NOT EXISTS download_request_tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL,
    item_key TEXT NOT NULL,
    source TEXT NOT NULL,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT NOT NULL,
    size INTEGER NOT NULL DEFAULT 0,
    quality TEXT NOT NULL DEFAULT '',
    quality_score REAL NOT NULL DEFAULT 0,
    duration INTEGER,
    backend_data TEXT,
    transfer_id TEXT,
    state TEXT NOT NULL DEFAULT 'queued',
    percent REAL NOT NULL DEFAULT 0,
    transferred INTEGER NOT NULL DEFAULT 0,
    speed REAL NOT NULL DEFAULT 0,
    error TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (request_id, item_key),
    FOREIGN KEY (request_id) REFERENCES download_requests(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_download_request_tracks_request
    ON download_request_tracks (request_id);
