# jgrep Explorer Herdr Plugin

Optional Herdr workflow package for opening `jgrep explore` inside a terminal workspace.

The plugin is intentionally thin. It launches the installed `jgrep` binary and does not execute filters itself. State includes temporary selected text and the last generated filter; configuration is separate from state by default.

## Local Development

```bash
herdr plugin link plugins/herdr-jgrep
herdr plugin action list --plugin jgrep.explorer
herdr plugin pane open --plugin jgrep.explorer --entrypoint explorer
```

## Actions

- `open`: launches `jgrep explore` for an explicit path, a path from Herdr context, or the first nearby JSON/YAML file in the current workspace.
- `filter-selection`: writes selected JSON/YAML text from Herdr context to a temporary file and opens it in `jgrep explore`.
- `install-completion`: prints shell completion install commands by default. Use `JGREP_COMPLETION_INSTALL=write` to install with backups.
- `open-error-logs`: opens nearby JSON/YAML log data with an initial `log.level=ERROR` shortcut filter.
- `print-last-filter`: prints the last generated jq filter recorded by a plugin-launched explorer.

## Configuration

The scripts work without configuration. Optional environment variables:

- `JGREP_BIN`: path to the `jgrep` binary, default `jgrep`.
- `HERDR_PLUGIN_CONFIG_DIR`: user-controlled config directory, default `${XDG_CONFIG_HOME:-$HOME/.config}/jgrep/herdr`.
- `HERDR_PLUGIN_STATE_DIR`: selected-text files, last filter and cleanup state, default `${XDG_STATE_HOME:-$HOME/.local/state}/jgrep/herdr`.
- `HERDR_PLUGIN_CONTEXT_JSON`: Herdr workspace, pane, and selection context.
- `JGREP_LAST_FILTER_FILE`: optional path where `jgrep explore` records the current generated jq filter.

Optional config file:

```toml
# $HERDR_PLUGIN_CONFIG_DIR/config.toml
jgrep_bin = "jgrep"
max_input_bytes = "52428800"
max_schema_documents = "500"
max_preview_results = "30"
completion_install = "print"
```

Environment variables override config values where both exist.

New config/state directories are created with mode `0700`; new state files use
an owner-only umask. Existing explicit directories must belong to the invoking
user and must not be writable by group or others. Existing configuration files
must have the same owner, must not be group/world-writable, and cannot be
symlinks. Leaf directory symlinks and writable/foreign-owned ancestor paths are
rejected before configuration reads, cleanup or execution. Root-owned sticky
temporary ancestors and trusted root-owned macOS aliases remain supported.

The old `${TMPDIR:-/tmp}/jgrep-herdr/config/config.toml` fallback is **never read
or migrated automatically**. To retain a legitimate old configuration, inspect
it and copy it yourself into the new private config directory. The scripts fail
closed with an `unsafe plugin path` diagnostic for unsafe overrides rather than
changing permissions on existing directories or trusting shared configuration.

Recommended Herdr keybinding:

```toml
[[keys.command]]
key = "prefix+j"
type = "plugin_action"
command = "jgrep.explorer.open"
description = "open jgrep explorer"
```
