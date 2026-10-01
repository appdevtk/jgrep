#!/usr/bin/env sh
set -eu

plugin_root=${HERDR_PLUGIN_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
. "$plugin_root/scripts/lib.sh"

jgrep_bin=$(jgrep_bin)
mode=${JGREP_COMPLETION_INSTALL:-$(config_value completion_install print)}
ensure_jgrep "$jgrep_bin"

print_commands() {
  cat <<EOF
# Bash
mkdir -p "\$HOME/.local/share/bash-completion/completions"
$jgrep_bin completion bash > "\$HOME/.local/share/bash-completion/completions/jgrep"

# Zsh
mkdir -p "\$HOME/.zsh/completions"
$jgrep_bin completion zsh > "\$HOME/.zsh/completions/_jgrep"

# Fish
mkdir -p "\$HOME/.config/fish/completions"
$jgrep_bin completion fish > "\$HOME/.config/fish/completions/jgrep.fish"
EOF
}

write_completion() {
  shell_name=${SHELL##*/}
  case "$shell_name" in
    bash)
      target="$HOME/.local/share/bash-completion/completions/jgrep"
      generator=bash
      ;;
    zsh)
      target="$HOME/.zsh/completions/_jgrep"
      generator=zsh
      ;;
    fish)
      target="$HOME/.config/fish/completions/jgrep.fish"
      generator=fish
      ;;
    *)
      echo "jgrep Explorer: unknown shell '$shell_name'; printing install commands instead." >&2
      print_commands
      return 0
      ;;
  esac

  mkdir -p "$(dirname "$target")"
  if [ -e "$target" ]; then
    cp "$target" "$target.bak"
    echo "jgrep Explorer: backed up existing completion to $target.bak" >&2
  fi
  "$jgrep_bin" completion "$generator" >"$target"
  echo "jgrep Explorer: installed $generator completion to $target"
}

case "$mode" in
  print)
    print_commands
    ;;
  write)
    write_completion
    ;;
  *)
    echo "jgrep Explorer: JGREP_COMPLETION_INSTALL must be 'print' or 'write'." >&2
    exit 2
    ;;
esac
