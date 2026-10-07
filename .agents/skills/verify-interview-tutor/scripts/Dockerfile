# Verification image for interview-tutor: the Linux userland the README
# requires, with locked crates prefetched so runs need no network. The build
# context is streamed by verify-tutor and holds only this file and the four
# Cargo manifests and lockfiles.
FROM ubuntu:24.04

ARG RUST_TOOLCHAIN=1.98.0
ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates build-essential make python3 neovim tmux rustup procps \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --home-dir /home/tester --shell /bin/bash tester \
    && mkdir /work /proof \
    && chown tester:tester /work /proof
USER tester
ENV HOME=/home/tester \
    CARGO_HOME=/home/tester/.cargo \
    RUSTUP_HOME=/home/tester/.rustup \
    PATH=/home/tester/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

RUN rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal \
    && rustup default "$RUST_TOOLCHAIN"

COPY --chown=tester:tester cli/ /home/tester/deps/cli/
COPY --chown=tester:tester rust/ /home/tester/deps/rust/
RUN set -eu; \
    for crate in cli rust; do \
        dir=/home/tester/deps/$crate; \
        mkdir -p "$dir/src/bin"; \
        touch "$dir/src/main.rs" "$dir/src/lib.rs" "$dir/src/bin/interview-tutor.rs"; \
        cargo fetch --locked --manifest-path "$dir/Cargo.toml"; \
    done; \
    rm -rf /home/tester/deps
