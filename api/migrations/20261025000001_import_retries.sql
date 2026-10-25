CREATE TABLE IF NOT EXISTS import_retries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL,
    item TEXT NOT NULL,
    job TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (username, item)
);

CREATE INDEX IF NOT EXISTS idx_import_retries_user ON import_retries (username, id);
