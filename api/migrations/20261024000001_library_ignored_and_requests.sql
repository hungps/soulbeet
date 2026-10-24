ALTER TABLE library_releases ADD COLUMN ignored INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS library_request_queue (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    folder_path TEXT NOT NULL,
    query_json TEXT NOT NULL,
    label TEXT NOT NULL,
    artist TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (user_id, folder_id, label, artist),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_library_request_queue_user
    ON library_request_queue(user_id, last_attempt_at);
