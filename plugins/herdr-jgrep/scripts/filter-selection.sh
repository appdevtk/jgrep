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

tmp=$(mktemp "$state_dir/selection-XXXXXX")
chmod 600 "$tmp"

selection_from_context() {
  [ -n "${HERDR_PLUGIN_CONTEXT_JSON:-}" ] || return 1
  command -v python3 >/dev/null 2>&1 || return 1
  python3 - <<'PY'
import json
import os
import sys

raw = os.environ.get("HERDR_PLUGIN_CONTEXT_JSON", "")
try:
    data = json.loads(raw)
except Exception:
    sys.exit(1)

keys = ("selected_text", "selection_text", "text")

def walk(node):
    if isinstance(node, dict):
        for key in keys:
            value = node.get(key)
            if isinstance(value, str) and value:
                print(value, end="")
                raise SystemExit(0)
        for value in node.values():
            walk(value)
    elif isinstance(node, list):
        for value in node:
            walk(value)

walk(data)
sys.exit(1)
PY
}

if selection_from_context >"$tmp"; then
  :
elif [ ! -t 0 ]; then
  cat >"$tmp"
else
  rm -f "$tmp"
  echo "jgrep Explorer: no selected text found in HERDR_PLUGIN_CONTEXT_JSON and no stdin was provided." >&2
  exit 2
fi

if [ ! -s "$tmp" ]; then
  rm -f "$tmp"
  echo "jgrep Explorer: selected text is empty." >&2
  exit 2
fi

check_file_size "$tmp" "$max_input_bytes"
export JGREP_LAST_FILTER_FILE=${JGREP_LAST_FILTER_FILE:-"$state_dir/last-filter.jq"}
exec "$jgrep_bin" explore \
  --max-input-bytes "$max_input_bytes" \
  --max-schema-documents "$max_schema_documents" \
  --max-preview-results "$max_preview_results" \
  "$tmp"
