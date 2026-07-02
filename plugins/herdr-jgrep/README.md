# jgrep Explorer Herdr Plugin

Optional Herdr workflow package for opening `jgrep explore` inside a terminal workspace.

The plugin is intentionally thin. It does not execute filters itself or store source data in plugin config; it launches the installed `jgrep` binary and uses `HERDR_PLUGIN_STATE_DIR` only for temporary selected-text files.

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
- `HERDR_PLUGIN_CONFIG_DIR`: user-editable plugin config directory.
- `HERDR_PLUGIN_STATE_DIR`: temporary selected-text files and cleanup state.
- `HERDR_PLUGIN_CONTEXT_JSON`: Herdr workspace, pane, and selection context.
- `JGREP_LAST_FILTER_FILE`: optional path where `jgrep explore` records the current generated jq filter.

Optional config file:

```toml
# $HERDR_PLUGIN_CONFIG_DIR/config.toml
jgrep_bin = "jgrep"
max_input_bytes = "52428800"
completion_install = "print"
```

Environment variables override config values where both exist.

Recommended Herdr keybinding:

```toml
[[keys.command]]
key = "prefix+j"
type = "plugin_action"
command = "jgrep.explorer.open"
description = "open jgrep explorer"
```
