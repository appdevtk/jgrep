#!/usr/bin/env sh
set -eu

plugin_root=${HERDR_PLUGIN_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
state_dir=${HERDR_PLUGIN_STATE_DIR:-${TMPDIR:-/tmp}/jgrep-herdr}
jgrep_bin=${JGREP_BIN:-jgrep}

mkdir -p "$state_dir"
find "$state_dir" -type f -name 'selection-*' -mtime +1 -delete 2>/dev/null || true

if ! command -v "$jgrep_bin" >/dev/null 2>&1; then
  echo "jgrep Explorer: cannot find jgrep. Set JGREP_BIN or install jgrep first." >&2
  exit 2
fi

if ! "$jgrep_bin" explore --help >/dev/null 2>&1; then
  echo "jgrep Explorer: this jgrep binary does not provide 'jgrep explore'." >&2
  exit 2
fi

context_value() {
  key=$1
  [ -n "${HERDR_PLUGIN_CONTEXT_JSON:-}" ] || return 1
  command -v python3 >/dev/null 2>&1 || return 1
  CONTEXT_KEY=$key python3 - <<'PY'
import json
import os
import sys

key = os.environ["CONTEXT_KEY"]
raw = os.environ.get("HERDR_PLUGIN_CONTEXT_JSON", "")
try:
    data = json.loads(raw)
except Exception:
    sys.exit(1)

def walk(node):
    if isinstance(node, dict):
        if key in node and isinstance(node[key], str) and node[key]:
            print(node[key])
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

workspace=$(context_value workspace_dir || context_value workspace || pwd)
target=${1:-}

if [ -z "$target" ]; then
  target=$(context_value selected_path || context_value file_path || context_value path || true)
fi

if [ -z "$target" ] && [ -d "$workspace" ]; then
  target=$(find "$workspace" -maxdepth 4 -type f \( -name '*.json' -o -name '*.ndjson' -o -name '*.yaml' -o -name '*.yml' \) 2>/dev/null | head -n 1 || true)
fi

if [ -z "$target" ]; then
  echo "jgrep Explorer: pass a JSON/YAML file path, select a path in Herdr, or pipe data to: jgrep explore" >&2
  echo "plugin root: $plugin_root" >&2
  exit 2
fi

export JGREP_LAST_FILTER_FILE=${JGREP_LAST_FILTER_FILE:-"$state_dir/last-filter.jq"}
exec "$jgrep_bin" explore "$target"
