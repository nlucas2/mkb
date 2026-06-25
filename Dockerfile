# syntax=docker/dockerfile:1
#
# Multi-stage build for mkb.
#
# Stages:
#   tester     – runs `cargo test --workspace` (native, on the build platform)
#   builder    – compiles mkbd (onnx embedder + vendored model, compiled in) + the client
#                binaries for the *target* platform (built natively / cross-compiled, so the
#                statically-linked onnxruntime resolves per-arch without cross-link pain)
#   runtime    – minimal Debian base for the daemon image
#   final      – the mkbd daemon image shipped to the registry (per --platform)
#   artifacts  – a scratch stage holding just the binaries, extracted to the host
#                via `docker buildx build --target artifacts --output type=local`
#
# Build deps live in each Rust stage; ca-certificates is needed for cargo and for ort's
# build-time onnxruntime download. The embedding model is vendored in-repo and compiled directly
# into the daemon binary (the default `vendored-model` feature), so the daemon runs the neural
# embedder fully offline and never downloads a model. ca-certificates is kept only for outbound
# TLS to a configured remote embeddings endpoint (opt-in) and general correctness.
#
# Base must be Debian trixie (not bookworm): ort's prebuilt static onnxruntime references
# newer glibc/libstdc++ symbols (e.g. __isoc23_strtol, __cxa_call_terminate) that bookworm's
# toolchain doesn't provide, so the daemon fails to link there.


# -- Test stage ---------------------------------------------------------------
FROM --platform=$BUILDPLATFORM rust:slim-trixie AS tester
# git is required by cargo-deny's advisories check: it clones the RustSec advisory-db (a git repo)
# at check time. `rust:slim-trixie` ships without it, so install it alongside the build basics.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# Cache the dependency download separately from source: copy every crate manifest, stub
# their sources, fetch, then bring in the real source.
COPY Cargo.toml Cargo.lock ./
COPY crates/mkb-core/Cargo.toml     crates/mkb-core/
COPY crates/mkb-index/Cargo.toml    crates/mkb-index/
COPY crates/mkb-embed/Cargo.toml    crates/mkb-embed/
COPY crates/mkb-protocol/Cargo.toml crates/mkb-protocol/
COPY crates/mkb-view/Cargo.toml     crates/mkb-view/
COPY crates/mkbd/Cargo.toml         crates/mkbd/
COPY crates/mkb-mcp/Cargo.toml      crates/mkb-mcp/
COPY crates/mkb-cli/Cargo.toml      crates/mkb-cli/
RUN mkdir -p \
        crates/mkb-core/src crates/mkb-index/src crates/mkb-embed/src \
        crates/mkb-protocol/src crates/mkb-view/src \
        crates/mkbd/src crates/mkb-mcp/src crates/mkb-cli/src \
    && touch \
        crates/mkb-core/src/lib.rs crates/mkb-index/src/lib.rs \
        crates/mkb-embed/src/lib.rs crates/mkb-protocol/src/lib.rs \
        crates/mkb-view/src/lib.rs \
    && for b in mkbd mkb-mcp mkb-cli; do echo 'fn main(){}' > crates/$b/src/main.rs; done \
    && mkdir -p crates/mkb-embed/examples \
    && echo 'fn main(){}' > crates/mkb-embed/examples/footprint.rs
RUN cargo fetch

COPY . .

# One gate: `cargo test --workspace` compiles the workspace and runs every test — including the
# in-process docs-as-data drift check (tests/docs_drift.rs), which re-exports the vault and asserts
# every generated doc still matches its source block. No separate build, daemon, or shell step.
RUN cargo test --workspace

# Supply-chain gate: cargo-deny enforces deny.toml — RustSec advisories (real vulnerabilities
# fail; known unmaintained transitive crates are acknowledged with reasons), a permissive-only
# license allowlist, and a crates.io-only source policy. Checked for the workspace AND the
# separate desktop-app workspace from the one shared config. A pinned prebuilt (static musl)
# binary keeps this fast — no from-source compile. `cargo fetch` for the app populates the index
# so its check resolves offline.
# renovate: datasource=github-releases depName=EmbarkStudios/cargo-deny
ARG CARGO_DENY_VERSION=0.19.9
RUN curl -sSL "https://github.com/EmbarkStudios/cargo-deny/releases/download/${CARGO_DENY_VERSION}/cargo-deny-${CARGO_DENY_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
      | tar -xz -C /usr/local/bin --strip-components=1 \
        "cargo-deny-${CARGO_DENY_VERSION}-x86_64-unknown-linux-musl/cargo-deny" \
    && cargo deny check \
    && cargo fetch --manifest-path app/mkb-tauri/src-tauri/Cargo.toml \
    && cargo deny --manifest-path app/mkb-tauri/src-tauri/Cargo.toml check --config deny.toml


# -- amd64 builder (native on the runner) -------------------------------------
FROM --platform=$BUILDPLATFORM rust:slim-trixie AS builder-amd64
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/mkb-core/Cargo.toml     crates/mkb-core/
COPY crates/mkb-index/Cargo.toml    crates/mkb-index/
COPY crates/mkb-embed/Cargo.toml    crates/mkb-embed/
COPY crates/mkb-protocol/Cargo.toml crates/mkb-protocol/
COPY crates/mkb-view/Cargo.toml     crates/mkb-view/
COPY crates/mkbd/Cargo.toml         crates/mkbd/
COPY crates/mkb-mcp/Cargo.toml      crates/mkb-mcp/
COPY crates/mkb-cli/Cargo.toml      crates/mkb-cli/
RUN mkdir -p \
        crates/mkb-core/src crates/mkb-index/src crates/mkb-embed/src \
        crates/mkb-protocol/src crates/mkb-view/src \
        crates/mkbd/src crates/mkb-mcp/src crates/mkb-cli/src \
    && touch \
        crates/mkb-core/src/lib.rs crates/mkb-index/src/lib.rs \
        crates/mkb-embed/src/lib.rs crates/mkb-protocol/src/lib.rs \
        crates/mkb-view/src/lib.rs \
    && for b in mkbd mkb-mcp mkb-cli; do echo 'fn main(){}' > crates/$b/src/main.rs; done \
    && mkdir -p crates/mkb-embed/examples \
    && echo 'fn main(){}' > crates/mkb-embed/examples/footprint.rs
RUN cargo fetch

COPY . .
# Build the daemon (semantic search baked in by default) and the thin clients in one shot. onnx is
# a daemon-only feature, so it never leaks into the clients' dep tree.
RUN cargo build --release -p mkbd -p mkb-cli -p mkb-mcp
# Normalise output location so final/artifacts stages are arch-agnostic.
RUN mkdir -p /out && cp target/release/mkbd target/release/mkb \
        target/release/mkb-mcp /out/


# -- arm64 builder (cross-compiled, native speed on the runner) ----------------
# Cross-compilation (not QEMU emulation) keeps the build fast. ort downloads a prebuilt
# aarch64 onnxruntime static lib (it is listed in ort's dist.txt), so onnxruntime is never
# compiled from source here — only linked with the cross toolchain. fastembed uses rustls
# (not openssl), so no libssl / dpkg arm64 cross-libs are needed.
FROM --platform=$BUILDPLATFORM rust:slim-trixie AS builder-arm64
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config ca-certificates \
    gcc-aarch64-linux-gnu g++-aarch64-linux-gnu libc6-dev-arm64-cross \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add aarch64-unknown-linux-gnu
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/mkb-core/Cargo.toml     crates/mkb-core/
COPY crates/mkb-index/Cargo.toml    crates/mkb-index/
COPY crates/mkb-embed/Cargo.toml    crates/mkb-embed/
COPY crates/mkb-protocol/Cargo.toml crates/mkb-protocol/
COPY crates/mkb-view/Cargo.toml     crates/mkb-view/
COPY crates/mkbd/Cargo.toml         crates/mkbd/
COPY crates/mkb-mcp/Cargo.toml      crates/mkb-mcp/
COPY crates/mkb-cli/Cargo.toml      crates/mkb-cli/
RUN mkdir -p \
        crates/mkb-core/src crates/mkb-index/src crates/mkb-embed/src \
        crates/mkb-protocol/src crates/mkb-view/src \
        crates/mkbd/src crates/mkb-mcp/src crates/mkb-cli/src \
    && touch \
        crates/mkb-core/src/lib.rs crates/mkb-index/src/lib.rs \
        crates/mkb-embed/src/lib.rs crates/mkb-protocol/src/lib.rs \
        crates/mkb-view/src/lib.rs \
    && for b in mkbd mkb-mcp mkb-cli; do echo 'fn main(){}' > crates/$b/src/main.rs; done \
    && mkdir -p crates/mkb-embed/examples \
    && echo 'fn main(){}' > crates/mkb-embed/examples/footprint.rs
RUN cargo fetch --target aarch64-unknown-linux-gnu

COPY . .
RUN cargo build --release --target aarch64-unknown-linux-gnu \
        -p mkbd -p mkb-cli -p mkb-mcp
RUN mkdir -p /out && cp \
        target/aarch64-unknown-linux-gnu/release/mkbd \
        target/aarch64-unknown-linux-gnu/release/mkb \
        target/aarch64-unknown-linux-gnu/release/mkb-mcp \
        /out/


# -- Runtime base (resolves to the target platform) ---------------------------
FROM debian:trixie-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
# Run as an ARBITRARY uid/gid chosen at deploy time (Kubernetes securityContext, `docker run
# -u`, OpenShift random uids, etc.) — no user is baked in. The image follows the "arbitrary
# user" convention: default to non-root uid 65534 (nobody), make the relevant paths owned by
# the root group (gid 0) and group-writable, and set HOME/dirs so any uid can create its
# runtime files.
WORKDIR /vault
# A group-writable vault dir, so whatever uid the container runs as can write the vault/index.
# gid 0 (root group) is always a supplemental group. The embedding model needs no files on disk
# — it is compiled into the daemon binary.
RUN chgrp -R 0 /vault \
    && chmod -R g=u /vault
ENV HOME=/vault
# Vault is mounted here; the local index is created under $MKB_VAULT/.mkb at runtime.
ENV MKB_VAULT=/vault
EXPOSE 7820
# Default to a non-root uid; the actual uid is overridable at deploy time and the image works
# with any uid (its gid 0 membership grants access to the prepared paths).
USER 65534:0
ENTRYPOINT ["mkbd"]
# Token must be provided at runtime (e.g. from a Secret) via $MKB_TOKEN.
CMD ["--vault", "/vault", "--listen", "0.0.0.0:7820"]


# -- Final daemon images (one per architecture) -------------------------------
# Build with `--platform linux/amd64 --target final-amd64` / `linux/arm64 final-arm64`.
# The runtime base inherits the target platform; the heavy compile already happened
# natively in the matching builder stage.
FROM runtime AS final-amd64
COPY --from=builder-amd64 /out/mkbd /usr/local/bin/mkbd

FROM runtime AS final-arm64
COPY --from=builder-arm64 /out/mkbd /usr/local/bin/mkbd


# -- Release payload, for extraction to the host (not runnable images) --------
# The daemon + clients. The daemon is self-contained — the embedding model is compiled into the
# binary — so no model files ship alongside. The container image is just one way to run mkbd;
# these tarballs are the native way (Linux service / CLI).
FROM scratch AS artifacts-amd64
COPY --from=builder-amd64 /out/mkbd   /mkbd
COPY --from=builder-amd64 /out/mkb     /mkb
COPY --from=builder-amd64 /out/mkb-mcp /mkb-mcp

FROM scratch AS artifacts-arm64
COPY --from=builder-arm64 /out/mkbd   /mkbd
COPY --from=builder-arm64 /out/mkb     /mkb
COPY --from=builder-arm64 /out/mkb-mcp /mkb-mcp
