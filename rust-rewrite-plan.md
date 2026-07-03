# Rust Migration Status for `jgrep`

## Summary

- The Rust implementation is now the primary and only implementation in this branch.
- The supported product surface is one CLI command: `jgrep`.
- `jgrep` handles JSON, NDJSON, YAML, and YAML multi-document input.
- YAML support that previously lived in a separate command is folded into `jgrep`.
- The former implementation modules and comparison-only golden tests have been removed from this branch.

## Current Architecture

- `crates/jgrep-core` contains CLI behavior, input parsing, discovery, matching, output formatting, shell completion, schema inference, shortcuts, and the interactive explorer.
- `crates/jgrep` is the release binary crate and integration-test host.
- The workspace root owns shared Cargo dependency versions and CI runs Rust formatting, linting, and tests.

## Supported Behavior

- `jgrep FILTER [FILE...]` works for JSON, NDJSON, YAML, and YAML multi-document inputs.
- Recursive search includes `.json`, `.yaml`, and `.yml`.
- Stdin is sniffed by content and can parse JSON, NDJSON, or YAML.
- `null` and `false` jq results are treated as non-matches.
- Text results are printed without JSON quotes.
- Non-text results are printed as compact JSON by default and pretty JSON with `--pretty`.
- Exit codes remain grep-like:
  - `0`: at least one match.
  - `1`: no matches.
  - `2`: invalid filter, parse error, IO error, or invalid CLI option combination.

## Validation Commands

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo bench --no-run
```

## Remaining Improvement Areas

- Keep broadening explorer interaction tests when new keybindings or view states are added.
- Track measured performance regressions with Criterion before optimizing internals.
- Document known `jaq` compatibility differences only when a user-visible case is confirmed.
