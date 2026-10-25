ALTER TABLE download_request_tracks ADD COLUMN local_path TEXT;
ALTER TABLE download_request_tracks ADD COLUMN import_output TEXT;
ALTER TABLE download_request_tracks ADD COLUMN retryable INTEGER NOT NULL DEFAULT 0;
ALTER TABLE download_request_tracks ADD COLUMN can_choose_import INTEGER NOT NULL DEFAULT 0;
ALTER TABLE download_request_tracks ADD COLUMN queue_position INTEGER;
