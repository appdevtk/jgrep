#!/usr/bin/env sh

# Newly written filters/history/selection state must remain private even when
# an explicit owner-controlled directory is searchable by other users.
umask 077

plugin_root() {
  if [ -n "${HERDR_PLUGIN_ROOT:-}" ]; then
    printf '%s\n' "$HERDR_PLUGIN_ROOT"
  else
    CDPATH= cd -- "$(dirname -- "$0")/.." && pwd
  fi
}

unsafe_path() {
  echo "jgrep Explorer: unsafe plugin path: $1" >&2
  exit 2
}

# Supported hosts have different stat interfaces. Do not follow a leaf symlink.
path_authority() {
  case "$(uname -s)" in
    Darwin) stat -f '%u %Lp' "$1" ;;
    Linux) stat -c '%u %a' "$1" ;;
    *) unsafe_path "unsupported host" ;;
  esac
}

check_directory_chain() (
  path=$1
  [ "$path" != / ] || return 0
  check_directory_chain "$(dirname "$path")" || exit 2
  if [ -L "$path" ]; then
    set -- $(path_authority "$path")
    [ "$1" = 0 ] || unsafe_path "$path (symlink)"
    # Root-owned aliases such as macOS /var are legitimate. Validate their
    # physical destination too; callers subsequently use only physical paths.
    physical=$(CDPATH= cd -P -- "$path" && pwd -P) || exit 2
    check_directory_chain "$physical"
  elif [ -e "$path" ]; then
    [ -d "$path" ] || unsafe_path "$path (not a directory)"
    set -- $(path_authority "$path")
    [ "$1" = 0 ] || [ "$1" = "$(id -u)" ] || unsafe_path "$path (foreign owner)"
    if [ "$((0$2 & 022))" -ne 0 ]; then
      [ "$1" = 0 ] && [ "$((0$2 & 01000))" -ne 0 ] || unsafe_path "$path (writable ancestor)"
    fi
  fi
)

private_directory() (
  path=$1
  case "$path" in /*) ;; *) path="$(pwd -P)/$path" ;; esac
  # Avoid lexical parent traversal and refuse existing leaf aliases.
  case "$path" in */../*|*/..|*/./*|*/.) unsafe_path "$path (noncanonical path)" ;; esac
  path=${path%/}
  [ -n "$path" ] && [ ! -L "$path" ] || unsafe_path "$path (symlink/root)"
  check_directory_chain "$path" || exit 2
  umask 077
  mkdir -p "$path" || exit 2
  # Recheck after creation, including a competing creator of a missing leaf.
  check_directory_chain "$path" || exit 2
  [ ! -L "$path" ] || unsafe_path "$path (symlink)"
  set -- $(path_authority "$path")
  [ "$1" = "$(id -u)" ] && [ "$((0$2 & 022))" -eq 0 ] || unsafe_path "$path (requires owner-controlled directory)"
  CDPATH= cd -P -- "$path" && pwd -P
)

state_dir() {
  private_directory "${HERDR_PLUGIN_STATE_DIR:-${XDG_STATE_HOME:-${HOME:?}/.local/state}/jgrep/herdr}"
}

config_dir() {
  private_directory "${HERDR_PLUGIN_CONFIG_DIR:-${XDG_CONFIG_HOME:-${HOME:?}/.config}/jgrep/herdr}"
}

config_value() {
  key=$1
  fallback=$2
  config_file="$(config_dir)/config.toml"
  [ ! -L "$config_file" ] || unsafe_path "$config_file (symlink)"
  if [ ! -f "$config_file" ]; then
    printf '%s\n' "$fallback"
    return
  fi
  authority=$(path_authority "$config_file")
  set -- $authority
  [ "$1" = "$(id -u)" ] && [ "$((0$2 & 022))" -eq 0 ] || unsafe_path "$config_file (untrusted owner/write permissions)"
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

max_schema_documents() {
  if [ -n "${JGREP_MAX_SCHEMA_DOCUMENTS:-}" ]; then
    printf '%s\n' "$JGREP_MAX_SCHEMA_DOCUMENTS"
  else
    config_value max_schema_documents 500
  fi
}

max_preview_results() {
  if [ -n "${JGREP_MAX_PREVIEW_RESULTS:-}" ]; then
    printf '%s\n' "$JGREP_MAX_PREVIEW_RESULTS"
  else
    config_value max_preview_results 30
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
