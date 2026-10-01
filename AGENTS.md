# Agent Instructions

## Working principles

- Keep changes focused and preserve unrelated user work.
- Follow the existing code style, project structure and documented workflows.
- Prefer explicit architecture, strong types and clear ownership boundaries.
- Use small, cohesive modules and existing helpers before adding abstractions.
- Handle expected failures explicitly. Avoid panics on untrusted input.
- Preserve supported behavior and compatibility unless a change is requested.

## Implementation and documentation

- Add regression tests for new behavior, bug fixes and important edge cases.
- Update the README for user-facing or operational changes.
- Document non-trivial logic and update architecture or flow diagrams when needed.
- Keep dependency changes deliberate, lockfiles consistent and toolchains pinned.
- Never commit credentials or private local configuration.

## Validation

For Rust changes, run the documented checks, including:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
```

- Check affected optional-feature configurations when changing shared code.
- For terminal UI changes, build the binary and run the documented PTY tests.
- For scripts, integrations and builds, use the relevant existing test or lint tools.
- Report what passed, what was not tested and any environment limitations.
- Do not claim live integration or release validation from local tests alone.

## Git and communication

- Use reviewable commits and respect the requested branch and history workflow.
- Back up existing history before an explicitly requested rewrite.
- Do not push, publish or perform unrelated destructive actions without authorization.
- Summarize outcomes, verification and remaining risks clearly and concisely.
