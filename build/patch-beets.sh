#!/bin/sh
# Patches the one crash in the pinned beets release (see requirements-beets.txt).
#
# beets 2.11 resolves a release's tracklist through get_tracks_from_medium,
# which reads recording["aliases"] unguarded. Releases with more than 500 tracks
# are re-fetched in chunks with browse_recordings, whose response carries no
# aliases for the nested recordings, so every metadata source that resolves a
# release - MusicBrainz and Acoustid alike - dies with KeyError: 'aliases' and
# the import matches nothing ("Lookup failed in Acoustid: 'aliases'").
#
# Fails when the line it patches is gone, so a beets bump forces a revisit
# instead of silently shipping the crash again.
set -eu

venv="${1:?usage: patch-beets.sh <venv>}"
file=$(ls "$venv"/lib/python3.*/site-packages/beetsplug/musicbrainz.py)
readonly='_preferred_alias(recording["aliases"])'
patched='_preferred_alias(recording.get("aliases", []))'

if grep -qF "$patched" "$file"; then
  exit 0
fi
if ! grep -qF "$readonly" "$file"; then
  echo "patch-beets: $file no longer matches the patched line" >&2
  exit 1
fi

sed "s|_preferred_alias(recording\[\"aliases\"\])|$patched|" "$file" >"$file.patched"
mv "$file.patched" "$file"
grep -qF "$patched" "$file"
