#!/usr/bin/env sh
set -eu

plugin_root=${HERDR_PLUGIN_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
. "$plugin_root/scripts/lib.sh"

state_dir=$(state_dir)
jgrep_bin=$(jgrep_bin)
max_input_bytes=$(max_input_bytes)
max_schema_documents=$(max_schema_documents)
max_preview_results=$(max_preview_results)
cleanup_old_selections "$state_dir"

ensure_jgrep "$jgrep_bin"

if ! "$jgrep_bin" explore --help >/dev/null 2>&1; then
  echo "jgrep Explorer: this jgrep binary does not provide 'jgrep explore'." >&2
  exit 2
fi

workspace=$(context_value workspace_dir || context_value workspace || pwd)
target=${1:-}

if [ -z "$target" ]; then
  target=$(context_value selected_path || context_value file_path || context_value path || true)
fi

if [ -z "$target" ] && [ -d "$workspace" ]; then
  target=$(first_data_file "$workspace" || true)
fi

if [ -z "$target" ]; then
  echo "jgrep Explorer: pass a JSON/YAML file path, select a path in Herdr, or pipe data to: jgrep explore" >&2
  echo "plugin root: $plugin_root" >&2
  exit 2
fi

check_file_size "$target" "$max_input_bytes"
export JGREP_LAST_FILTER_FILE=${JGREP_LAST_FILTER_FILE:-"$state_dir/last-filter.jq"}
exec "$jgrep_bin" explore \
  --max-input-bytes "$max_input_bytes" \
  --max-schema-documents "$max_schema_documents" \
  --max-preview-results "$max_preview_results" \
  "$target"
