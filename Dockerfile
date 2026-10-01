# syntax=docker/dockerfile:1
FROM rust:1.96.0-alpine@sha256:f87aa870663e2b57ec8c69de82c7eedf7383bee987eef7612c0359635eaadb41 AS build
RUN apk add --no-cache build-base
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ ./crates/
COPY plugins/k9s-jgrep/plugins.yaml ./plugins/k9s-jgrep/plugins.yaml
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --locked --release -p jgrep && \
    ./target/release/jgrep --version && \
    printf '%s\n' '{"name":"container-build"}' | ./target/release/jgrep '.name' && \
    readelf -l ./target/release/jgrep > /tmp/jgrep-program-headers && \
    ! grep -q INTERP /tmp/jgrep-program-headers

# Export only the standalone musl binary, without the toolchain or build cache.
FROM scratch AS binary
COPY --from=build /src/target/release/jgrep /jgrep
