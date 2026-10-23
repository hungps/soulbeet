-- Per-user media-server links. A user can keep Navidrome and Plex
-- configured at the same time; `users.media_server` selects the active one.
CREATE TABLE IF NOT EXISTS media_server_configs (
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    url TEXT,
    secure INTEGER NOT NULL DEFAULT 0,
    token TEXT,
    account_id TEXT,
    account_name TEXT,
    server_id TEXT,
    server_name TEXT,
    status TEXT NOT NULL DEFAULT 'unknown',
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, kind),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- Translate a local library path to the path the media server sees (and
-- back). Plex containers frequently mount `/music` under `/volume1/music`.
CREATE TABLE IF NOT EXISTS media_path_mappings (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    local_path TEXT NOT NULL,
    remote_path TEXT NOT NULL,
    UNIQUE (user_id, kind, local_path),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO media_server_configs (user_id, kind, token, account_id, status)
SELECT id, media_server, media_token, media_account_id, media_status
FROM users
WHERE media_token IS NOT NULL;
