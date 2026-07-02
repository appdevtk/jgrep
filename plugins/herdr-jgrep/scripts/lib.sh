#!/usr/bin/env sh

plugin_root() {
  if [ -n "${HERDR_PLUGIN_ROOT:-}" ]; then
    printf '%s\n' "$HERDR_PLUGIN_ROOT"
  else
    CDPATH= cd -- "$(dirname -- "$0")/.." && pwd
  fi
}

state_dir() {
  printf '%s\n' "${HERDR_PLUGIN_STATE_DIR:-${TMPDIR:-/tmp}/jgrep-herdr}"
}

config_dir() {
  printf '%s\n' "${HERDR_PLUGIN_CONFIG_DIR:-$(state_dir)/config}"
}

config_value() {
  key=$1
  fallback=$2
  config_file="$(config_dir)/config.toml"
  if [ ! -f "$config_file" ]; then
    printf '%s\n' "$fallback"
    return
  fi
  value=$(awk -F= -v key="$key" '
    $1 ~ "^[[:space:]]*" key "[[:space:]]*$" {
      value=$2
      sub(/^[[:space:]]*/, "", value)
      sub(/[[:space:]]*$/, "", value)
      sub(/^"/, "", value)
      sub(/"$/, "", value)
      print value
      exit
    }
  ' "$config_file")
  printf '%s\n' "${value:-$fallback}"
}

jgrep_bin() {
  if [ -n "${JGREP_BIN:-}" ]; then
    printf '%s\n' "$JGREP_BIN"
  else
    config_value jgrep_bin jgrep
  fi
}

max_input_bytes() {
  if [ -n "${JGREP_MAX_INPUT_BYTES:-}" ]; then
    printf '%s\n' "$JGREP_MAX_INPUT_BYTES"
  else
    config_value max_input_bytes 52428800
  fi
}

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
        value = node.get(key)
        if isinstance(value, str) and value:
            print(value)
            raise SystemExit(0)
        for child in node.values():
            walk(child)
    elif isinstance(node, list):
        for child in node:
            walk(child)

walk(data)
sys.exit(1)
PY
}

ensure_jgrep() {
  bin=$1
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "jgrep Explorer: cannot find jgrep. Set JGREP_BIN or configure jgrep_bin." >&2
    exit 2
  fi
}

check_file_size() {
  file=$1
  limit=$2
  size=$(wc -c <"$file" | tr -d ' ')
  if [ "$size" -gt "$limit" ]; then
    echo "jgrep Explorer: $file is $size bytes, larger than max_input_bytes=$limit." >&2
    echo "Raise max_input_bytes in HERDR_PLUGIN_CONFIG_DIR/config.toml or choose a smaller sample." >&2
    exit 2
  fi
}

first_data_file() {
  root=$1
  find "$root" -maxdepth 4 -type f \( -name '*.json' -o -name '*.ndjson' -o -name '*.yaml' -o -name '*.yml' \) 2>/dev/null | sort | head -n 1
}

cleanup_old_selections() {
  dir=$1
  mkdir -p "$dir"
  find "$dir" -type f -name 'selection-*' -mtime +1 -delete 2>/dev/null || true
}
