# Herdr Plugin Plan

## Goal

Build an optional Herdr plugin that makes `jgrep` easier to use directly inside a terminal workspace.

The plugin should not replace the normal `jgrep` binary. It should wrap the existing Rust CLI and add a Herdr-native workflow for exploring JSON, NDJSON, and YAML with schema hints, shortcut filters, and live preview.

## Why Herdr Helps

Herdr plugins are executable workflow packages with a `herdr-plugin.toml` manifest. They can expose actions, terminal panes, keybindings, link handlers, and event hooks. That maps well to the next `jgrep` UX layer:

- `jgrep` already has a good scripted CLI path.
- Exploration needs a long-lived terminal surface, not another one-shot command.
- Herdr panes give us a place for an interactive `jgrep explore` TUI without changing normal shell behavior.
- Herdr actions can launch common workflows from the current workspace or selected text.

## Non-Goals

- Do not make Herdr required for `jgrep`.
- Do not move filter execution into the plugin.
- Do not invent a separate query language in the plugin.
- Do not implement a web UI.
- Do not store source data in Herdr plugin state.

## Implemented Package

The first plugin directory is in place:

```text
plugins/herdr-jgrep/
  herdr-plugin.toml
  README.md
  scripts/
    open-explorer.sh
    filter-selection.sh
    install-completion.sh
```

The first version can be shell-based because the real work stays in the `jgrep` binary. If the workflow grows, replace the scripts with a small Rust helper binary.

## Manifest Sketch

```toml
id = "jgrep.explorer"
name = "jgrep Explorer"
version = "0.1.0"
min_herdr_version = "0.7.0"
description = "Explore JSON, NDJSON, and YAML with jgrep inside Herdr"
platforms = ["linux", "macos"]

[[actions]]
id = "open"
title = "Open jgrep explorer"
contexts = ["workspace"]
command = ["scripts/open-explorer.sh"]

[[actions]]
id = "filter-selection"
title = "Filter selected JSON/YAML"
contexts = ["workspace"]
command = ["scripts/filter-selection.sh"]

[[actions]]
id = "install-completion"
title = "Install jgrep shell completion"
contexts = ["workspace"]
command = ["scripts/install-completion.sh"]

[[panes]]
id = "explorer"
title = "jgrep"
placement = "split"
command = ["jgrep", "explore"]
```

Windows can come later after the scripts are replaced or mirrored with PowerShell. Herdr supports Windows command launching, but a Unix-only first slice is less brittle.

## User Workflows

### Optional Keybinding

Herdr keybindings live in the user's Herdr config, not inside the plugin manifest. Recommended binding:

```toml
[[keys.command]]
key = "prefix+j"
type = "plugin_action"
command = "jgrep.explorer.open"
description = "open jgrep explorer"
```

That keeps the plugin portable while still making the workflow one keystroke inside the terminal workspace.

### Open Explorer For A File

Expected command:

```bash
herdr plugin action invoke jgrep.explorer.open
```

Behavior:

- Detect the current workspace directory from Herdr context.
- If the focused pane exposes selected text or a path-like token, prefer that as input.
- Otherwise open a picker-like fallback in the pane:
  - recent `*.json`, `*.ndjson`, `*.yaml`, `*.yml`
  - stdin instructions
  - manual path entry
- Start `jgrep explore <file>` in a Herdr split pane.

### Explore Command Output

Expected user flow:

```bash
kubectl get pods -o yaml | jgrep explore
```

Inside Herdr, the plugin action can open a pane with a prepared command template. The user can edit and run it, which keeps command ownership clear.

### Filter Selected Text

If Herdr context includes selected text from the focused pane:

1. Write selected text to a temporary file under `HERDR_PLUGIN_STATE_DIR`.
2. Launch `jgrep explore <tmp-file>` in a plugin pane.
3. Remove old temporary files opportunistically on next invocation.

This makes copied API responses, log snippets, and Kubernetes YAML immediately inspectable.

### Install Completion

The action should run:

```bash
jgrep completion bash
jgrep completion zsh
jgrep completion fish
```

and install to common shell completion directories where safe. If the target shell is unknown, print exact commands instead of mutating shell files.

## `jgrep explore` Contract

The plugin depends on the standalone `jgrep explore` subcommand. Keep that subcommand useful without Herdr:

```bash
jgrep explore file.json
cat logs.ndjson | jgrep explore
```

Current TUI behavior:

- Parse JSON, NDJSON, and YAML through the same code path as normal `jgrep`.
- Infer a compact schema from the first N documents or the whole file when small.
- Show a field list and live output preview.
- Accept both shortcuts and jq:
  - `name`
  - `status=active`
  - `age>=18`
  - `.items[] | select(.active == true)`
- Show the generated jq filter for shortcut expressions.
- Let the user print the filtered result or export the final jq expression.

## Plugin Runtime Rules

Use Herdr environment variables instead of hardcoded paths:

- `HERDR_BIN_PATH` to call Herdr itself.
- `HERDR_PLUGIN_ROOT` for bundled scripts.
- `HERDR_PLUGIN_CONFIG_DIR` for user-editable plugin configuration.
- `HERDR_PLUGIN_STATE_DIR` for temp files and recent input metadata.
- `HERDR_PLUGIN_CONTEXT_JSON` for workspace, pane, selected text, and focused pane data.

Do not store durable user config inside the plugin source directory, because GitHub-installed plugin roots are managed checkouts.

## Configuration

Optional config file:

```text
$HERDR_PLUGIN_CONFIG_DIR/config.toml
```

Suggested fields:

```toml
jgrep_bin = "jgrep"
default_placement = "split"
max_schema_documents = 500
completion_install = "print"
```

Defaults should work without a config file.

## Implementation Slices

### Slice 1: Local Plugin Skeleton

- Added `plugins/herdr-jgrep/herdr-plugin.toml`.
- Added `open-explorer.sh` that validates `jgrep` exists and opens `jgrep explore`.
- Added plugin README with local link command:

```bash
herdr plugin link plugins/herdr-jgrep
herdr plugin action list --plugin jgrep.explorer
herdr plugin pane open --plugin jgrep.explorer --entrypoint explorer
```

Validation:

- Manifest links successfully.
- Pane opens and reports a helpful error if `jgrep explore` is not available yet.

### Slice 2: Implement `jgrep explore`

- Added `explore` subcommand in Rust.
- Used `ratatui` and `crossterm`.
- Shared parsing and filter shortcut code with non-interactive mode.
- Added snapshot-like tests for schema inference and shortcut-to-jq generation.

Validation:

- `jgrep explore fixtures/sample.json` renders a non-empty TUI.
- `cat fixtures/sample.yaml | jgrep explore` works.
- Existing CLI behavior remains unchanged.

### Slice 3: Herdr Context Integration

- Parse `HERDR_PLUGIN_CONTEXT_JSON`.
- Support selected text to temp-file workflow.
- Support path/token detection from focused pane context when present.
- Keep pane opening delegated to Herdr manifest entrypoints until the exact Herdr pane invocation contract is available.

Validation:

- Selected JSON opens in explorer.
- Selected YAML opens in explorer.
- Temp files stay under `HERDR_PLUGIN_STATE_DIR`.

### Slice 4: Completion And Convenience Actions

- Added shell-aware completion installer.
- Added common actions:
  - Explore current file or nearby JSON/YAML file.
  - Explore selected text.
  - Open logs with a `log.level=ERROR` filter.
  - Print generated jq filter from the last plugin-launched explorer session.

Validation:

- Completion installer does not overwrite existing files without a clear backup or print-only mode.
- Actions fail with actionable terminal output.

## Architecture Notes

- Keep `jgrep-core` responsible for parsing, filter compilation, shortcut expansion, and schema inference.
- Keep `jgrep` binary responsible for CLI and TUI entrypoints.
- Keep Herdr plugin scripts thin and disposable.
- If scripts start accumulating logic, move them into `crates/jgrep-herdr-helper` or fold the behavior into `jgrep herdr ...`.

## Risks

- Herdr plugin APIs are intentionally broad and unsandboxed; users should inspect the manifest and scripts before installing.
- A pane action cannot magically know arbitrary shell pipeline history; selected text and explicit file paths are more reliable.
- Large files need streaming or sampling. First TUI slice should sample for schema and only preview a bounded result set.
- Windows support should wait until the plugin avoids POSIX shell assumptions.

## Recommendation

Do it as an optional plugin plus a standalone `jgrep explore` subcommand.

That gives Herdr users the terminal-native experience without coupling the core tool to one terminal workspace manager. The first useful milestone is:

1. `jgrep explore <file>` works outside Herdr.
2. `herdr plugin pane open --plugin jgrep.explorer --entrypoint explorer` opens it inside Herdr.
3. Selected text can be sent into the explorer through a plugin action.
