#!/usr/bin/env sh
set -eu

plugin_root=${HERDR_PLUGIN_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
. "$plugin_root/scripts/lib.sh"

state_dir=$(state_dir)
filter_file=${JGREP_LAST_FILTER_FILE:-"$state_dir/last-filter.jq"}

if [ ! -s "$filter_file" ]; then
  echo "jgrep Explorer: no generated jq filter has been recorded yet." >&2
  exit 1
fi

cat "$filter_file"
