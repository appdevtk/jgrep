# jgrep UX Simplification Notes

## Current friction

- jq is powerful, but the syntax is high-friction for common field extraction and equality filters.
- YAML support makes `jgrep file.yaml` useful as a converter/viewer, not only as a grep-like matcher.
- Log workflows often use dotted literal keys such as `log.level`, while jq requires quoting those keys as `.["log.level"]`.
- Exploration is a different job than scripted filtering: users need schema hints, field discovery, and live feedback before they know the final filter.

## Implemented low-friction aliases

- `jgrep file.json` and `jgrep file.yaml` default to identity filter `.`.
- `jgrep -p FIELD file` expands to a jq field extraction.
  - `-p name` -> `.name`
  - `-p user.name` checks direct key `user.name` first, then nested `.user.name`.
- `jgrep -w TEST file` expands to a jq `select(...)`.
  - `-w status=active`
  - `-w age>=18`
  - `-w log.level=ERROR`
- `-w` can be repeated and combines conditions with `and`.
- `-p` and `-w` can be combined:

```bash
jgrep -w status=active -p name users.ndjson
jgrep -C -w log.level=ERROR logs.ndjson
```

## Implemented next UX layer: `jgrep explore`

An interactive TUI is available as a separate subcommand instead of changing non-interactive CLI semantics:

```bash
jgrep explore file.json
kubectl get pods -o yaml | jgrep explore
```

Current layout:

- Left pane: schema-derived field list.
- Top input: editable filter field.
- Right pane: live result preview.
- Bottom hint bar: schema-derived field suggestions, shortcuts, and current output count.

Current behavior:

- Load JSON/NDJSON/YAML using the same parser path as CLI mode.
- Infer a compact schema from the first N documents or the full file when small.
- Autocomplete field paths from observed schema with `Tab`.
- Support both jq and shortcut expressions:
  - `name`
  - `status=active`
  - `age>=18`
  - full jq when prefixed with `.` or `select(...)`
- Export the final filter or preview result to stdout:
  - `Enter`: print result.
  - `Ctrl-Y`: print generated jq filter.
  - `Ctrl-S`: save generated jq filter to a file.

Non-interactive output, including tests and redirected stdout, prints a deterministic schema and preview snapshot with `--print`.

## External UX references

- [`fx`](https://fx.wtf/) positions itself as an interactive terminal JSON viewer and processor with JSON streaming support.
- [`jless`](https://jless.io/) focuses on reading, exploring, searching, and collapsing JSON in the terminal.
- [`jqp`](https://github.com/noahgorstein/jqp) is a TUI playground for running jq queries interactively against JSON/NDJSON input.
- [`jnv`](https://github.com/ynqa/jnv) combines an interactive JSON viewer, jq filter editor, syntax highlighting, completions, and `jaq`.
- [`jiq`](https://lib.rs/crates/jiq) emphasizes real-time output and autocomplete for jq functions and fields.
- The official [jq playground](https://play.jqlang.org/) uses a query pane, input pane, options, and output preview; that is a good conceptual model for terminal mode.

## Implementation notes

The first TUI slice uses `ratatui` plus `crossterm`.

Implemented first slice:

1. Parse one file or stdin into memory.
2. Show schema-derived field list.
3. Let the user type a filter or shortcut.
4. Re-run the filter as input changes.
5. Show output preview and final generated jq.

Still deferred: collapsible tree navigation, large-file virtualization, editing source data, persistent config, themes, and advanced keymaps.
