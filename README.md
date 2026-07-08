# jgrep

`grep` for structured data — filter and search JSON, NDJSON, and YAML files using [jq](https://jqlang.github.io/jq/) expressions.

| Tool | Input | Recursive discovery |
|------|-------|---------------------|
| `jgrep` | JSON, NDJSON, YAML | `*.json`, `*.yaml`, `*.yml` |

```bash
# Extract a field from JSON
jgrep '.name' users.json

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

**Or build from source:**

```bash
git clone https://github.com/subnix-work/jgrep.git
cd jgrep

cargo build --release -p jgrep
./target/release/jgrep '.name' file.json
./target/release/jgrep '.metadata.name' manifest.yaml
```

The release profile is optimized for small native binaries with LTO and stripped symbols.

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
  -r, --recursive            Recurse into directories (searches *.json, *.yaml, *.yml files)
  -l, --files-with-matches   Only print filenames that contain matches
  -c, --count                Print match count per file
  -s, --slurp                Collect all results into a single JSON array
  -n, --null-input           Use null as input (no file needed; evaluate filter directly)
  -f, --from-file=FILE       Read filter expression from a file
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
- `Ctrl-O` - switch between Filter and Output
- `Ctrl-G` - toggle wide log preview and hide/show the field schema
- `Ctrl-F` - toggle pretty JSON output
- `Ctrl-L` - toggle log-level color for printed output
- `Ctrl-K` - toggle colors off/on
- `Up`/`Down` - scroll fields while editing Filter, scroll preview while editing Output or wide logs
- `PageUp`/`PageDown` or mouse wheel - scroll preview
- `Enter` - print current results
- `Ctrl-Y` - print the generated jq expression

### VHS terminal demos

The same scenarios are checked in as [VHS](https://terminaltrove.com/vhs/) tapes:

```bash
mkdir -p demos/out
vhs demos/tapes/quickstart.tape
vhs demos/tapes/yaml-and-recursive.tape
vhs demos/tapes/shortcuts-and-slurp.tape
vhs demos/tapes/counts-files-null.tape
vhs demos/tapes/pretty-and-custom-color.tape
vhs demos/tapes/explorer-snapshot.tape
vhs demos/tapes/streaming-highlight.tape
vhs demos/tapes/explore-tui-streaming.tape
```

Generated GIFs are written to `demos/out/`.

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
