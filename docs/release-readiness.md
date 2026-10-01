# Release validation

Use the pinned Rust toolchain and committed Cargo lockfile. Release instructions
and artifact verification are documented in [RELEASING.md](../RELEASING.md).

## Required checks

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo build --locked -p jgrep --all-features
python3 scripts/test-tui.py target/debug/jgrep
python3 -m unittest discover -s demos/scripts -p 'test_*.py'
```

Also check the supported optional-feature configurations:

```sh
for features in minimal yaml completion explore explore,yaml; do
  cargo test --locked -p jgrep --no-default-features --features "$features" || exit
done
cargo audit --deny warnings
docker build --check .
docker build --output type=local,dest=dist .
```

Validate GitHub workflow syntax with actionlint. Exercise the exported Docker
binary on Linux with the matching CPU architecture. For recording validation,
follow [demos/README.md](../demos/README.md).

## Coverage and limitations

The full-feature suite covers parsing, field shortcuts, stdin and stream errors,
output failures, schema completion, terminal-safe display and plugin boundaries.
Real PTY tests cover editing, save/export, help, resizing, streaming and normal
terminal-mode restoration. k9s process tests use synthetic inputs and stub
kubectl processes; Herdr tests exercise its launch scripts.

Local tests do not establish live cluster access, RBAC, k9s keybindings inside
the real application, shell-profile completion installation, or compatibility
with every terminal emulator. Initialization-error cleanup is enforced by a
terminal guard, but an injected initialization-failure runtime test is not
available. Benchmark smoke tests do not establish throughput or latency targets.

## Publication checklist

- Run target CI for Linux x64 and macOS x64/arm64; verify downloaded release
  assets rather than treating local builds as equivalent.
- Verify the final binaries using their published `SHA256SUMS`.
- Update both AUR packages with the final tagged source/binary hashes and test
  with `makepkg` on Arch Linux. Do not publish packages using `SKIP` checksums.
- Select the release version, update release notes and follow the signed-tag
  workflow in [RELEASING.md](../RELEASING.md).
