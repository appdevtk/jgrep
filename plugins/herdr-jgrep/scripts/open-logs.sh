#!/usr/bin/env sh
set -eu

state_dir=${HERDR_PLUGIN_STATE_DIR:-${TMPDIR:-/tmp}/jgrep-herdr}
jgrep_bin=${JGREP_BIN:-jgrep}
mkdir -p "$state_dir"

if ! command -v "$jgrep_bin" >/dev/null 2>&1; then
  echo "jgrep Explorer: cannot find jgrep. Set JGREP_BIN or install jgrep first." >&2
  exit 2
fi

workspace=${1:-$(pwd)}
target=$(find "$workspace" -maxdepth 4 -type f \( -name '*.json' -o -name '*.ndjson' -o -name '*.yaml' -o -name '*.yml' \) 2>/dev/null | head -n 1 || true)

if [ -z "$target" ]; then
  echo "jgrep Explorer: no JSON/YAML log-like file found under $workspace." >&2
  exit 2
fi

export JGREP_LAST_FILTER_FILE=${JGREP_LAST_FILTER_FILE:-"$state_dir/last-filter.jq"}
exec "$jgrep_bin" explore --filter 'log.level=ERROR' "$target"
