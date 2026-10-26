ALTER TABLE library_tracks ADD COLUMN audio_format TEXT;
ALTER TABLE library_tracks ADD COLUMN bitrate INTEGER;
ALTER TABLE library_tracks ADD COLUMN bit_depth INTEGER;
ALTER TABLE library_tracks ADD COLUMN sample_rate INTEGER;

ALTER TABLE library_releases ADD COLUMN audio_format TEXT;
ALTER TABLE library_releases ADD COLUMN bitrate INTEGER;
ALTER TABLE library_releases ADD COLUMN bit_depth INTEGER;
ALTER TABLE library_releases ADD COLUMN sample_rate INTEGER;
ALTER TABLE library_releases ADD COLUMN quality_varies INTEGER NOT NULL DEFAULT 0;
