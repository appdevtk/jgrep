#!/usr/bin/env sh
set -eu

state_dir=${HERDR_PLUGIN_STATE_DIR:-${TMPDIR:-/tmp}/jgrep-herdr}
filter_file=${JGREP_LAST_FILTER_FILE:-"$state_dir/last-filter.jq"}

if [ ! -s "$filter_file" ]; then
  echo "jgrep Explorer: no generated jq filter has been recorded yet." >&2
  exit 1
fi

cat "$filter_file"
