# jgrep

`grep` for structured data — filter and search JSON, NDJSON, and YAML files using [jq](https://jqlang.github.io/jq/) expressions.

| Tool | Input | Recursive discovery |
|------|-------|---------------------|
| `jgrep` | JSON, NDJSON, JSON Lines, YAML | `*.json`, `*.ndjson`, `*.jsonl`, `*.yaml`, `*.yml` |

```bash
# Extract a field from JSON
jgrep '.name' users.json

# Explicit filter, with unambiguous input paths
jgrep --filter '.name' users.json

# Filter YAML manifests
jgrep '.spec.template.spec.containers[].image' deploy/

# Search recursively, count matches
jgrep -rc 'select(.status == "active")' ./data/

# Pipe from stdin
curl -s https://api.example.com/users | jgrep 'select(.role == "admin")'
kubectl get deploy checkout -o yaml | jgrep '.spec.replicas'

# Explore fields and preview filters interactively
jgrep explore users.json
kubectl get pods -o yaml | jgrep explore
```

## Installation

Download native binaries from the [Releases](https://github.com/subnix-work/jgrep/releases) page.

Linux x64 and macOS release assets provide the `jgrep` binary.
Verify downloaded assets with the release's `SHA256SUMS` file; see
[SECURITY.md](SECURITY.md) for the command and [RELEASING.md](RELEASING.md) for
the maintainer checklist.

**Or build from source:**

```bash
git clone https://github.com/subnix-work/jgrep.git
cd jgrep

cargo build --release -p jgrep
./target/release/jgrep '.name' file.json
./target/release/jgrep '.metadata.name' manifest.yaml
```

The release profile is optimized for small native binaries with LTO and stripped symbols.

### Build using Docker only

No Rust installation on the host is required. From the repository root:

```bash
docker build --output type=local,dest=dist .
# On Linux with the matching CPU architecture:
./dist/jgrep '.name' file.json
```

The multi-stage [Dockerfile](Dockerfile) uses a digest-pinned Rust 1.96.0 Alpine
builder, `Cargo.lock`, both workspace crates and the embedded k9s plugin template.
It smoke-tests the binary and checks that it has no dynamic ELF interpreter.
Only the static musl Linux executable is exported to `dist/jgrep`; Rust and the
build tools stay inside Docker. Cargo download caches are kept by BuildKit.

The default architecture follows the Docker builder. For an x64 Linux target,
use `docker build --platform linux/amd64 --output type=local,dest=dist .`; the
builder must support that platform (native or emulated). Similarly use
`linux/arm64` for ARM64. These are Linux binaries, not native macOS executables.
Run `docker build --check .` for Dockerfile checks. CI builds and exercises the
exported binary on Linux x64 without installing Rust in that job.

### Native macOS Docker build and k9s installation

On a Mac, use Docker Desktop and Apple's Command Line Tools (`xcode-select
--install` if absent). No host Rust or Python installation is required:

```bash
bash scripts/build-macos.sh
# Build, install missing k9s/kubectl through Homebrew, and install the plugin:
bash scripts/build-macos.sh --install
# Safely replace an existing managed plugin, keeping filters and a backup:
bash scripts/build-macos.sh --install --update
```

The [macOS Dockerfile](Dockerfile.macos) uses the digest-pinned Rust toolchain's
bundled LLVM linker and the locally installed Apple SDK as a read-only named
build context. The SDK is not copied into image layers or exported artifacts.
The output is a native Mach-O executable at `dist/macos-arm64/jgrep` or
`dist/macos-x86_64/jgrep`, not a Linux executable. The native output is smoke-tested
on the host. `--arch arm64|x86_64` also supports building for another Mac; that
architecture must be runtime-tested on its target. Installation requires the
host's native architecture.

Use `--config-dir /path/from/k9s-info` for a custom k9s configuration. Otherwise
the helper honors `K9S_CONFIG_DIR` and `XDG_CONFIG_HOME`, then the macOS default.
Existing k9s/kubectl installations are reused, other plugins are preserved, and
`--update` backs up the old snapshot outside scanned plugin directories. Restart
k9s after installation. `Shift-J` opens logs with editable Filter and Output;
`Ctrl-G` enlarges Preview without stopping ingestion. See the
[k9s integration guide](plugins/k9s-jgrep/README.md) for limits and tests.

### Optional source features

Optional features can be disabled for smaller binaries:

| Build | Command | Linux x64 size |
|-------|---------|----------------|
| Minimal JSON/NDJSON CLI | `cargo build --release -p jgrep --no-default-features` | 1.8M / 1,858,008 bytes |
| Minimal + YAML | `cargo build --release -p jgrep --no-default-features --features yaml` | 1.9M / 1,935,192 bytes |
| Minimal + completion | `cargo build --release -p jgrep --no-default-features --features completion` | 1.9M / 1,926,312 bytes |
| Minimal + explore TUI | `cargo build --release -p jgrep --no-default-features --features explore` | 2.1M / 2,150,992 bytes |
| Minimal + explore TUI + YAML | `cargo build --release -p jgrep --no-default-features --features explore,yaml` | 2.2M / 2,228,496 bytes |
| Full CLI, YAML, completion, explore TUI | `cargo build --release -p jgrep` | 2.2M / 2,294,352 bytes |

## Usage

### jgrep

```
Usage: jgrep [-rclsnhV] [-f=FILE] [-p EXPR] [-w TEST] [--pretty] [-C|--color-level] [--color-level-field FIELD] [--no-color] [FILTER] [FILE...]

  FILTER   jq filter expression (e.g. '.name', 'select(.age > 18)', '.items[]').
           Defaults to '.' when omitted or when the first positional argument is an existing path.
  FILE     JSON, NDJSON, or YAML files to search. Reads from stdin if omitted.

Options:
  -r, --recursive            Recurse into directories (searches *.json, *.ndjson, *.jsonl, *.yaml, *.yml files)
  -l, --files-with-matches   Only print filenames that contain matches
  -c, --count                Print match count per file
  -s, --slurp                Collect all results into a single JSON array
  -n, --null-input           Use null as input (no file needed; evaluate filter directly)
  -f, --from-file=FILE       Read filter expression from a file
      --filter=EXPR          Explicit jq filter, positional arguments are input paths
  -p, --path=FIELD           Extract a dotted field path without jq syntax
  -w, --where=TEST           Filter with a simple condition (e.g. status=active, age>=18)
      --pretty               Pretty-print JSON output
  -C, --color-level          Color each output line by log level
      --color-level-field    Field used by --color-level (e.g. app.level)
      --no-color             Disable colored output (also respects $NO_COLOR)
  -h, --help                 Show this help message
  -V, --version              Print version

Subcommands:
  explore [FILE]             Explore JSON, NDJSON, or YAML with schema hints and a live preview
  completion SHELL           Generate shell completion script
```

## Examples

```bash
# All users older than 18
jgrep 'select(.age > 18)' users.ndjson

# Extract nested field
jgrep '.address.city' customers.json

# Extract a field without jq syntax
jgrep -p address.city customers.json

# Filter without jq syntax
jgrep -w status=active -p name users.ndjson
jgrep -w 'age>=18' -p name users.ndjson

# Convert JSON/YAML to JSON with the default identity filter
jgrep config.yaml
cat config.yaml | jgrep

# Explore before committing to a jq expression
jgrep explore config.yaml
jgrep explore --filter status=active logs.ndjson
jgrep explore --max-input-bytes 10485760 --max-preview-results 20 logs.ndjson

# Find files containing errors
jgrep -l 'select(.level == "ERROR")' logs/*.json

# Color structured logs by their level field
jgrep -C logs.ndjson
jgrep -C -w log.level=ERROR logs.ndjson

# Count active items per file
jgrep -rc 'select(.active == true)' ./data/

# Array items matching condition
jgrep '.orders[] | select(.total > 100)' orders.json

# Collect all names into a JSON array
jgrep -s '.name' users.ndjson

# Aggregate across multiple files
jgrep -s '.price' products/*.json | jgrep -n 'add / length'

# Evaluate an expression without input
jgrep -n 'now | strftime("%Y-%m-%d")'

# Use a multi-line filter stored in a file
jgrep -f filter.jq events.ndjson

# Query a Kubernetes manifest
jgrep '.spec.template.spec.containers[].image' deployment.yaml

# Find all Deployments with more than 2 replicas
jgrep -r 'select(.kind == "Deployment" and .spec.replicas > 2)' k8s/

# Pipe kubectl output
kubectl get deploy checkout -o yaml | jgrep '.spec.template.spec.containers[].image'
```

When `explore --filter` and `--where` are combined, the `--where` conditions
select input records before the filter and Output expression run. Invalid
conditions return exit code 2. Printing results from a stream with parse or
input-limit errors preserves valid results and returns exit code 2.

Use `--filter EXPR` when a jq expression could also be a local filename.
With `--filter`, `--path`, or `--from-file`, every positional argument is an
input path, including missing paths. Choose one of these three filter sources,
then add `--where` conditions if needed. Plain `.` and `..` are jq expressions,
while `jgrep -r .` searches the current directory. To view a file named like a
subcommand, use its path, for example `jgrep ./explore`.

Running `jgrep` in a terminal without arguments shows help and examples.
Piped input continues to use the default identity filter. A command with a
filter but no file or pipe reports how to provide input or use `--null-input`.

## Demo commands

The repository includes fixtures under `demos/fixtures/` so the main features can be tested without external services:

```bash
cargo build -q -p jgrep
alias jgrep=./target/debug/jgrep

# jq field extraction and filtering
jgrep '.name' demos/fixtures/users.ndjson
jgrep 'select(.age >= 18) | .name' demos/fixtures/users.ndjson

# Shortcut filters without writing full jq
jgrep -w status=active -p name demos/fixtures/users.ndjson
jgrep -f demos/fixtures/filter.jq demos/fixtures/users.ndjson

# YAML and recursive mixed-format search
jgrep '.spec.template.spec.containers[].image' demos/fixtures/k8s/deployment.yaml
jgrep -r 'select(.kind == "Deployment" and .spec.replicas > 2) | .metadata.name' demos/fixtures/k8s

# Slurp, color-level logs, explorer snapshot, completion
jgrep -s '.role' demos/fixtures/users.ndjson
jgrep -C -w log.level=ERROR demos/fixtures/logs.ndjson
jgrep explore --print --filter status=active demos/fixtures/users.ndjson
jgrep completion bash | sed -n '1,12p'

# Simulated kubectl logs stream with level highlighting
JGREP_DEMO_DELAY=0.35 demos/scripts/k8s-log-stream.sh |
  jgrep -C -f demos/fixtures/k8s-log-line.jq

# Live explore TUI from streaming JSON logs
JGREP_DEMO_DELAY=0.45 demos/scripts/k8s-log-stream.sh |
  jgrep explore
```

Inside the TUI, `Filter` selects records and `Output` controls what each match prints. Use `Ctrl-O` to switch between them:

```text
Filter: log.level=ERROR
Output: message
```

Simple field comparisons accept `=` or `==`, including completed jq lookups:
`level=ERROR`, `.level=ERROR` and `.level == "ERROR"` select the same records.
Use `select(...)` for complex jq predicates. Field filters such as `log` and
`.log` both transform the input to the nested object before applying Output.
Raw jq pipelines remain available. The normal CLI keeps raw jq semantics:
use `--where level==ERROR` or `select(.level == "ERROR")` to select records.

The same split is available at startup. This filters records first, then prints only the selected output expression:

```sh
jgrep explore -w log.level=ERROR -p message -C logs.ndjson
```

Use jq object or array expressions when you want multiple fields in the output:

```sh
jgrep explore -w log.level=ERROR -p '{time: .timestamp, level: .log.level, message, trace_id}' logs.ndjson
jgrep explore -w log.level=ERROR -p '[.timestamp, .log.level, .message]' logs.ndjson
```

Useful TUI toggles:

- `F1` - open/close keyboard help with examples for Filter and Output
- `Tab` - complete an observed field as a literal jq lookup (for example `.name`)
- `Ctrl-O` - switch between Filter and Output
- `Ctrl-G` - toggle wide log preview and hide/show the field schema
- `Ctrl-F` - toggle pretty JSON output
- `Ctrl-L` - toggle log-level color for printed output
- `Ctrl-K` - toggle colors off/on
- `Ctrl-T` - pause/resume the live tail (ingestion and export continue)
- `Up`/`Down` - scroll visible fields while editing Filter, otherwise scroll preview
- `PageUp`/`PageDown` or mouse wheel - scroll preview
- `Enter` - print current results
- `Ctrl-Y` - print the generated jq expression
- `Ctrl-S` - save the generated jq expression to `jgrep-filter.jq` (override with `JGREP_FILTER_SAVE_PATH`)
- `Esc` - quit without printing, or close help when it is open

Below 80 columns, the field panel is automatically hidden so the preview keeps
the full terminal width. The compact footer keeps `F1`, print, and quit visible.
Widening the terminal restores the field panel unless wide-preview mode is on.
In help, use arrows, PageUp/PageDown, Home/End, or the mouse wheel to scroll.
`F1` or `Esc` returns to the unchanged editor. Editing, saving, and printing
shortcuts are paused while help is open, and `Ctrl-C` still quits.

Long preview lines wrap without losing JSON indentation. Scrolling, live-tail
position and PageUp/PageDown use the rendered terminal rows and viewport size,
including colored logs and resizing. Unicode editor movement and deletion keep
whole graphemes intact, including combined accents and joined emoji. The field
panel's document ratio refers to the bounded schema sample, not the full stream.
Empty previews distinguish waiting for logs, paused display, disabled preview,
incompatible records and a completed query with no matches.

`NO_COLOR` disables colors on startup, as does `--no-color`. Inside the TUI,
`Ctrl-K` is an explicit override for display colors. Log badges remain textual
when enabled so severity is not conveyed through color alone. Raw result export
continues to honor `NO_COLOR`. The terminal renderer owns all preview colors;
there is no separate overlay that can overwrite wrapped rows or borders.

### Input and display safety

Streaming previews follow the latest matches in arrival order, bounded by
`--max-preview-results`. Scrolling upward pauses the displayed preview; `Ctrl-T`
resumes it. The match count includes all retained matches, not just the displayed
window. Changing Filter, Output or formatting rebuilds the preview and resumes
the tail. Buffered files and `--print` show the first matching results instead.
`Enter` exports all currently accepted matching records, including those outside
the preview window. It does not wait for future records from a live producer.

For a known NDJSON producer, `jgrep explore --stream-json` opens the TUI before
the first record arrives. Filter, Output, help and cancellation are usable during
the initial wait. The k9s log bridge selects this mode automatically; general
stdin autodetection still waits for the first line to distinguish JSON from YAML.
The flag cannot be combined with an input file or `--print`.

Streaming handles at most 512 events per UI cycle, without an idle delay while
a full batch is pending. Unchanged filters evaluate only new records, and schema
sampling stops growing at `--max-schema-documents`. Filter or formatting changes
rebuild the preview from retained records. Buffered and streaming exploration share terminal cleanup, including
initialization failures. These changes do not add a general jq execution timeout.

An incompatible record (for example an array passed to a string operation) is
skipped without clearing valid preview results. The UI reports evaluation errors;
export writes valid results and reports skipped errors on stderr with exit code
2. Syntax errors still block save/print/jq export until corrected. Normal CLI
evaluation also continues after per-record errors and returns exit code 2.
As with grep-style jq matching, `false` and `null` results are not matches.

JSON parsing rejects nesting beyond 128 containers with a controlled input error,
including file/stdin/NDJSON detection and streaming. This limit applies before
schema inference; `--max-schema-depth` only controls the displayed schema.
jaq's existing comments and byte-string syntax remain supported. Excessively
nested k9s log lines remain plain message text.

Observed-field completion preserves literal key identity, including dotted,
quoted and unusual keys; it never treats a field name as a jq program. Nested
and literal dotted keys can appear as separate candidates. Invalid UTF-8 keys
disable schema completion with a diagnostic rather than crashing exploration;
manual projections of valid fields remain available. Manually authored jq
is still supported and remains trusted code: expensive or infinite expressions
are not sandboxed or generally execution-budgeted.

Live previews and `explore --print` encode terminal control characters before
applying jgrep's own styles. Explicit raw CLI extraction and `Enter` result
export keep their existing raw-string behavior; avoid displaying untrusted raw
exports directly in a terminal. See the [Herdr configuration safety rules](plugins/herdr-jgrep/README.md)
for private defaults and the intentionally unsupported legacy `/tmp` config.

### k9s integration

From a selected pod in k9s, `Shift-J` opens live logs in jgrep; selected containers
use `Shift-K`. These bindings intentionally take precedence over built-in actions
in those views and are unique across the plugin file.
`Shift-U` explores the selected resource as JSON. Mixed JSON/text logs are
supported, the selected context is preserved, and no cluster writes are made.
The installer preserves existing plugins and keeps a private binary snapshot.
The bridge and installer are native Rust: `jgrep k9s`, no Python required.
See [installation, shortcuts and limits](plugins/k9s-jgrep/README.md).

### VHS terminal demos

The same scenarios are checked in as [VHS](https://terminaltrove.com/vhs/) tapes:

```bash
mkdir -p demos/out
# Or build, validate, and render the complete set:
bash scripts/render-demos.sh

# Individual recordings:
vhs demos/tapes/quickstart.tape
vhs demos/tapes/yaml-and-recursive.tape
vhs demos/tapes/shortcuts-and-slurp.tape
vhs demos/tapes/counts-files-null.tape
vhs demos/tapes/pretty-and-custom-color.tape
vhs demos/tapes/explorer-snapshot.tape
vhs demos/tapes/streaming-highlight.tape
vhs demos/tapes/explore-tui-streaming.tape
vhs demos/tapes/cli-entry-and-recovery.tape
vhs demos/tapes/explorer-complete-save-export.tape
vhs demos/tapes/explorer-help-and-resize.tape
```

All eleven demos generate GIF and MP4 files in `demos/out/`. The new recordings
also show explicit filters and missing-file recovery, autocomplete/save/export,
F1 help, and narrow-terminal reflow. See [demo coverage and recording notes](demos/README.md)
for the feature matrix and the demo-only F1/resize adapter.

In headless containers where Chromium cannot use its sandbox, prefix the render command with `VHS_NO_SANDBOX=1`.

### Shell completion

```bash
# Bash
jgrep completion bash > ~/.local/share/bash-completion/completions/jgrep

# Zsh
jgrep completion zsh > ~/.zsh/completions/_jgrep

# Fish
jgrep completion fish > ~/.config/fish/completions/jgrep.fish

# PowerShell
jgrep completion powershell >> $PROFILE
```

### Human-readable Kubernetes / ECS logs

Kubernetes and ECS logs are often NDJSON: one JSON object per line. You can filter structured fields and project each match into plain text:

```bash
# Turn JSON log events into readable text lines
cat ecs.ndjson | jgrep \
  '"\(.["@timestamp"]) [\(.["log.level"])] \(.["service.name"])/\(.kubernetes.pod): \(.message)"'

# Filter specific objects and unpack nested tracking fields
cat ecs.ndjson | jgrep \
  'select(.["log.level"] == "ERROR" and .kubernetes.namespace == "shop") |
   "\(.["@timestamp"]) ERROR \(.["service.name"]): \(.message) trace=\(.custom_tracker.trace_id)"'

# Color whole output lines by log level
cat ecs.ndjson | jgrep --color-level \
  '"[\(.["log.level"])] \(.["@timestamp"]) \(.message) trace=\(.custom_tracker.trace_id)"'

# Use a custom level field
cat app.ndjson | jgrep --color-level --color-level-field app.level \
  '"[\(.app.level)] \(.message)"'
```

## Filter syntax

`jgrep` uses [`jaq`](https://github.com/01mf02/jaq) for jq-compatible filter execution. JSON/NDJSON and YAML documents are each filtered independently.

**Exit codes** (same as `grep`):
- `0` — at least one match found
- `1` — no matches found
- `2` — error (invalid filter, file not found)

## What's new in 1.3.0

- **Rust-only `jgrep`** — one native binary handles JSON, NDJSON, YAML, and YAML multi-document input.
- **Interactive explorer** — `jgrep explore` shows inferred fields, live jq generation, preview output, completion, history, and save/export shortcuts.
- **Native CI and packaging** — Cargo builds, tests, release artifacts, and AUR metadata now target the Rust workspace directly.

## Unreleased maintenance changes

- Update Cargo dependencies, including jaq, clap, YAML parsing, and Criterion.
- Discover `.ndjson` and `.jsonl` files during recursive searches.
- Fix YAML stdin detection for keys starting with JSON scalar prefixes.
- Validate explorer `--where` conditions and combine them with `--filter`.
- Preserve buffered log records when the explorer starts streaming, enforce
  input limits while reading, and report stream errors when printing results.
- Report buffered output failures and generate downloadable release checksums
  using asset filenames.
- Test every supported feature build and the real interactive explorer before
  publishing a tagged release.
- Add explicit `--filter`, protect shortcut input paths from being ignored,
  and treat `.`/`..` correctly as jq expressions. Show examples on startup
  without arguments and recovery guidance when input is missing.

## Development checks

Use the pinned toolchain from `rust-toolchain.toml` and the committed lockfile:

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo build --locked -p jgrep --all-features
python3 scripts/test-tui.py target/debug/jgrep
cargo build --locked --release -p jgrep
```

The terminal test requires Python 3 and a Unix pseudo-terminal with access to
`/dev/tty`. It checks editing, save/export, output toggles, quit, and piped log
streams, including malformed and oversized input. The Rust suite also tests the
Herdr launch scripts. CI runs tests for each feature combination in the build
table above and audits dependencies with `cargo audit --deny warnings`.

The suites include fixed-seed randomized Filter/Output and CLI
where/path/count/slurp matrices, live-tail pause/resume through a synthetic k9s
producer, mixed-type recovery, and a 50,000-record PTY streaming regression.
Seeds are fixed for reproducibility; these tests do not execute arbitrary or
unbounded jq programs and do not establish live cluster/RBAC compatibility.

See [development notes](docs/development.md) for module boundaries and explorer
behavior, and [release validation](docs/release-readiness.md) for required checks,
validation limitations and the publication checklist.

## Core features

- **`-s` / `--slurp`** — collect all matching results from all files into one JSON array
- **`-n` / `--null-input`** — evaluate a filter without reading any file
- **`-f` / `--from-file`** — load the filter expression from a `.jq` file
- **Shell completion** — `jgrep completion bash|zsh|fish|powershell`
- **`--color-level`** — color output lines by log level field
- **Syntax highlighting** — JSON output is colorized when writing to a terminal
- **Better error messages** — parse errors include line and column number

## Tech stack

- Rust + [clap](https://docs.rs/clap/) — CLI framework
- [jaq](https://github.com/01mf02/jaq) — jq-compatible filter engine
- [yaml_serde](https://crates.io/crates/yaml_serde) — YAML parsing

## License

[Apache-2.0](LICENSE)
