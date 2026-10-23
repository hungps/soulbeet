-- Media-server abstraction: a user is linked to either Navidrome (Subsonic)
-- or Plex (plex.tv account token). The legacy navidrome_* columns are kept
-- for rollback safety; new code reads the media_* columns.
ALTER TABLE users ADD COLUMN media_server TEXT NOT NULL DEFAULT 'navidrome';
ALTER TABLE users ADD COLUMN media_token TEXT;
ALTER TABLE users ADD COLUMN media_status TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE users ADD COLUMN media_account_id TEXT;

UPDATE users SET media_token = navidrome_token, media_status = navidrome_status;
