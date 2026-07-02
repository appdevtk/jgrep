# Rust-Rewrite Plan for `jgrep`

## Summary

- Implement a parallel Rust port next to the existing Java/Quarkus implementation until feature and behavior parity is proven by tests.
- The future product surface is one CLI command: `jgrep`.
- `jgrep` must automatically handle JSON, NDJSON, and YAML. The current `ygrep` behavior is folded into `jgrep` instead of shipping a second primary command.
- Keep the externally visible behavior of the current tools where applicable: jq filters, output formatting, recursive discovery, grep-style exit codes, color behavior, and shell completions.
- Use `jaq` as the native Rust jq engine. Compare against the current Java implementation with golden tests so compatibility differences are visible early.

## Target Behavior

- `jgrep FILTER [FILE...]` works for JSON, NDJSON, YAML, and YAML multi-document inputs.
- File type detection:
  - `.json` files are parsed as JSON/NDJSON.
  - `.yaml` and `.yml` files are parsed as YAML.
  - stdin is sniffed by content: try JSON/NDJSON first, then YAML as fallback.
  - Recursive search includes `.json`, `.yaml`, and `.yml`.
- `null` and `false` jq results are treated as non-matches, matching the current Java behavior.
- All other jq results count as matches, including `0`, empty strings, empty arrays, and empty objects.
- Text results are printed without JSON quotes.
- Non-text results are printed as compact JSON by default and pretty JSON with `--pretty`.
- Slurp mode prints one JSON array of all matching results.
- Exit codes remain grep-like:
  - `0`: at least one match.
  - `1`: no matches.
  - `2`: invalid filter, parse error, IO error, or invalid CLI option combination.
- If an error happens after earlier matches were printed, keep the printed matches and return exit code `2`.

## CLI Scope

- Preserve the current options:
  - `-r`, `--recursive`
  - `-l`, `--files-with-matches`
  - `-c`, `--count`
  - `-s`, `--slurp`
  - `-n`, `--null-input`
  - `-f`, `--from-file`
  - `--pretty`
  - `--color-level`
  - `--color-level-field`
  - `--no-color`
  - `-h`, `--help`
  - `-V`, `--version`
- Reject `--count` together with `--files-with-matches` with exit code `2`.
- Keep `jgrep completion bash|zsh|fish|powershell`.
- Do not make `ygrep` a primary Rust binary.
- Optional migration compatibility: provide `ygrep` as a temporary alias or symlink to `jgrep`, but document `jgrep` as the only supported command.

## Implementation Plan

- Add a Cargo workspace at the repo root.
- Suggested crates:
  - `crates/jgrep-core`: shared runtime logic.
  - `crates/jgrep`: the release binary.
- Keep the existing Maven modules in place during the migration.
- Implement `jgrep-core` with focused modules:
  - `cli`: argument parsing and validation.
  - `discovery`: file expansion, recursive traversal, supported extensions, deterministic sorting.
  - `input`: JSON/NDJSON, YAML, stdin sniffing, and source labels.
  - `matcher`: jq filter compilation and execution via `jaq`.
  - `output`: string/non-string formatting, pretty output, slurp output, filename prefixes.
  - `color`: JSON syntax color and log-level color.
  - `error`: consistent user-facing error formatting and exit-code mapping.
- Compile the jq filter once per invocation and reuse it for all inputs.
- Use buffered stdout and stderr.
- Stream input where practical. Slurp mode may buffer matching results by design.
- Convert YAML inputs to JSON-compatible values before jq execution.

## Dependencies

- Runtime:
  - `clap` for CLI parsing.
  - `jaq` or the lower-level `jaq-*` crates for jq evaluation.
  - `serde_json` for JSON values and output.
  - `yaml_serde` for YAML parsing.
  - `walkdir` for recursive discovery.
  - `anstream`/`anstyle` or equivalent for terminal-aware color output.
- Tests:
  - `assert_cmd` for CLI integration tests.
  - `predicates` for output assertions.
  - `tempfile` for fixture directories.
  - `criterion` for performance benchmarks.

## Test Plan

- Keep the current Java tests as the reference suite.
- Port the existing `jgrep` and `ygrep` behavior tests into Rust integration tests against the single `jgrep` binary.
- Add golden tests that run equivalent fixtures against:
  - current Java `jgrep` for JSON/NDJSON behavior.
  - current Java `ygrep` for YAML behavior.
  - new Rust `jgrep`.
- Compare exit code, stdout, and stderr where practical.
- Required scenarios:
  - Field access.
  - `select(...)`.
  - Array iteration.
  - Nested field access.
  - Missing fields returning no match.
  - String interpolation.
  - Fallback values with `//`.
  - JSON files.
  - NDJSON files.
  - YAML files.
  - YAML multi-document files.
  - stdin JSON detection.
  - stdin YAML detection.
  - Recursive mixed JSON/YAML search.
  - Missing file.
  - Directory without `-r`.
  - Multiple files with filename prefixes.
  - `-c`.
  - `-l`.
  - invalid `-cl`.
  - `-s`.
  - `-s` with no matches prints `[]` and returns `1`.
  - `-n`.
  - `-f`.
  - Parse error after a previous match returns `2`.
  - Help output.
  - Version output.
  - Completion output for bash, zsh, fish, and PowerShell.
  - `--color-level`.
  - `--color-level-field`.
  - `--no-color`.
  - `NO_COLOR`.
  - TTY and non-TTY color behavior.

## Performance Plan

- First reach behavior parity, then optimize.
- Benchmark with `criterion`:
  - 10,000 NDJSON documents.
  - Large NDJSON file with streaming processing.
  - YAML multi-document input.
  - Recursive search across many mixed JSON/YAML files.
  - Slurp mode with many matches.
- Optimization priorities:
  - Compile filters once.
  - Avoid full-file reads where streaming works.
  - Minimize conversions between parser values, jq values, and output values.
  - Use buffered IO.
  - Only add allocator changes such as `mimalloc` if benchmarks show a clear benefit.

## Architecture Review Checklist

- `jgrep` and YAML support must not duplicate whole command implementations.
- Format detection is centralized and unit tested.
- Parser-specific code stays behind small adapters.
- Matching logic is independent from file discovery and output.
- Output formatting is deterministic and tested directly.
- Color mapping is isolated and tested.
- Error handling has one user-facing formatting path.
- No speculative plugin, strategy, or configuration systems until a real second implementation requires them.
- Known `jaq` compatibility differences are either fixed, covered by tests, or documented before replacing the Java implementation.

## Migration Steps

1. Add the Rust workspace and a minimal `jgrep` binary.
2. Implement JSON/NDJSON filtering for one file.
3. Add CLI flags and output behavior for the existing `jgrep` test cases.
4. Add YAML parsing and merge `ygrep` behavior into `jgrep`.
5. Add recursive mixed-format discovery.
6. Add stdin sniffing.
7. Add completion generation.
8. Add golden comparison tests against the Java binaries.
9. Run architecture review and fix issues found in the touched Rust code.
10. Run performance benchmarks and optimize measured bottlenecks.
11. Update README, release packaging, Docker, and AUR metadata for one Rust `jgrep` binary.
12. Remove or archive Java/Quarkus modules only after the Rust port passes parity gates.

## Validation Commands

```bash
mvn -q test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo bench
```

## Assumptions

- The rewrite is implemented as a parallel port first, not as an immediate hard replacement.
- The final supported CLI is one command: `jgrep`.
- `ygrep` is not a new primary binary; at most it is a temporary compatibility alias.
- `jaq` is the planned jq engine.
- `libjq` bindings are reconsidered only if golden tests expose critical jq compatibility gaps.
- The existing Java implementation remains the reference implementation during the migration.
