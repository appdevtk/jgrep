#!/usr/bin/env sh
set -eu

state_dir=${HERDR_PLUGIN_STATE_DIR:-${TMPDIR:-/tmp}/jgrep-herdr}
jgrep_bin=${JGREP_BIN:-jgrep}
mkdir -p "$state_dir"
find "$state_dir" -type f -name 'selection-*' -mtime +1 -delete 2>/dev/null || true

if ! command -v "$jgrep_bin" >/dev/null 2>&1; then
  echo "jgrep Explorer: cannot find jgrep. Set JGREP_BIN or install jgrep first." >&2
  exit 2
fi

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

export JGREP_LAST_FILTER_FILE=${JGREP_LAST_FILTER_FILE:-"$state_dir/last-filter.jq"}
exec "$jgrep_bin" explore "$tmp"
