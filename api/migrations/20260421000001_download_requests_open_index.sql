-- One open request session per user and release. Completed requests are
-- excluded so the next download for the same release starts a new session.
CREATE UNIQUE INDEX IF NOT EXISTS idx_download_requests_open
    ON download_requests (user_id, request_key)
    WHERE state IN ('active', 'stopped');
