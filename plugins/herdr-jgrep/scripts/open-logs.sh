#!/usr/bin/env sh
set -eu

plugin_root=${HERDR_PLUGIN_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
. "$plugin_root/scripts/lib.sh"

state_dir=$(state_dir)
jgrep_bin=$(jgrep_bin)
max_input_bytes=$(max_input_bytes)
max_schema_documents=$(max_schema_documents)
max_preview_results=$(max_preview_results)
mkdir -p "$state_dir"
ensure_jgrep "$jgrep_bin"

workspace=$(context_value workspace_dir || context_value workspace || pwd)
target=${1:-}

if [ -z "$target" ]; then
  target=$(context_value selected_path || context_value file_path || context_value path || true)
fi

if [ -z "$target" ] && [ -d "$workspace" ]; then
  target=$(find "$workspace" -maxdepth 4 -type f \( -name '*log*.json' -o -name '*log*.ndjson' -o -name '*.ndjson' -o -name '*.json' -o -name '*.yaml' -o -name '*.yml' \) 2>/dev/null | sort | head -n 1 || true)
fi

if [ -z "$target" ]; then
  echo "jgrep Explorer: no JSON/YAML log-like file found under $workspace." >&2
  exit 2
fi

check_file_size "$target" "$max_input_bytes"
export JGREP_LAST_FILTER_FILE=${JGREP_LAST_FILTER_FILE:-"$state_dir/last-filter.jq"}
exec "$jgrep_bin" explore \
  --max-input-bytes "$max_input_bytes" \
  --max-schema-documents "$max_schema_documents" \
  --max-preview-results "$max_preview_results" \
  --filter 'log.level=ERROR' \
  "$target"
