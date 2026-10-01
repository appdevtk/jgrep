#!/bin/bash
# Thin host orchestration: compilation stays in Docker, installation in Rust.
set -euo pipefail
usage() {
  printf '%s\n' 'Usage: bash scripts/build-macos.sh [--arch arm64|x86_64] [--install] [--update] [--config-dir PATH]'
}
if [[ "$(uname -s)" != Darwin ]]; then
  printf '%s\n' 'This helper requires macOS, Docker and the Apple Command Line Tools SDK.' >&2
  exit 2
fi
arch="$(uname -m)"
install=false
update=false
config_dir="${K9S_CONFIG_DIR:-${XDG_CONFIG_HOME:-${HOME}/Library/Application Support}/k9s}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch|--config-dir)
      if [[ $# -lt 2 || -z "$2" ]]; then usage >&2; exit 2; fi
      if [[ "$1" == --arch ]]; then arch="$2"; else config_dir="$2"; fi
      shift 2 ;;
    --install) install=true; shift ;;
    --update) update=true; shift ;;
    --help|-h) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done
case "$arch" in
  arm64) target=aarch64-apple-darwin ;;
  x86_64) target=x86_64-apple-darwin ;;
  *) printf '%s\n' 'Unsupported macOS architecture; choose arm64 or x86_64.' >&2; exit 2 ;;
esac
if $update && ! $install; then
  printf '%s\n' '--update requires --install.' >&2; exit 2
fi
if $install && [[ "$arch" != "$(uname -m)" ]]; then
  printf '%s\n' 'Installation requires the native host architecture.' >&2; exit 2
fi
command -v docker >/dev/null || { printf '%s\n' 'Install and start Docker Desktop first.' >&2; exit 2; }
sdk="${JGREP_MACOS_SDK:-$(xcrun --sdk macosx --show-sdk-path)}"
[[ -f "$sdk/usr/lib/libSystem.tbd" ]] || { printf '%s\n' 'No usable Apple SDK; run xcode-select --install.' >&2; exit 2; }
repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"
docker build -f Dockerfile.macos --build-arg "MACOS_TARGET=$target" \
  --build-context "macos-sdk=$sdk" --output "type=local,dest=dist/macos-$arch" .
binary="$repo/dist/macos-$arch/jgrep"
if [[ "$arch" == "$(uname -m)" ]]; then
  "$binary" --version
  result="$(printf '%s\n' '{"name":"macos-docker-build"}' | "$binary" '.name')"
  [[ "$result" == macos-docker-build ]]
fi
if $install; then
  missing=()
  for tool in k9s kubectl; do command -v "$tool" >/dev/null || missing+=("$tool"); done
  if [[ ${#missing[@]} -gt 0 ]]; then
    command -v brew >/dev/null || { printf '%s\n' 'Install k9s and kubectl, or Homebrew, before --install.' >&2; exit 2; }
    brew install "${missing[@]}"
  fi
  options=(k9s install --config-dir "$config_dir")
  if $update; then options+=(--update); fi
  "$binary" "${options[@]}"
  printf '%s\n' 'Restart k9s, then Shift-J opens streaming logs; F1 shows the TUI shortcuts.'
fi
