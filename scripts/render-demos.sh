#!/usr/bin/env bash
# Run from the repository root. Stops on the first failed screen assertion.
set -euo pipefail

for program in cargo python3 vhs ttyd ffmpeg ffprobe; do
  if ! command -v "$program" >/dev/null 2>&1; then
    printf 'Missing demo prerequisite: %s\n' "$program" >&2
    exit 1
  fi
done

cargo build --locked -p jgrep
python3 demos/scripts/test_explorer_recording.py
vhs validate demos/tapes/*.tape

for tape in demos/tapes/*.tape; do
  vhs "$tape"
done

python3 demos/scripts/verify_recordings.py
python3 demos/scripts/test_verify_recordings.py
