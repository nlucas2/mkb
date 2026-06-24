# syntax=docker/dockerfile:1
#
# Multi-stage build for mdkb.
#
# Stages:
#   tester     – runs `cargo test --workspace` (native, on the build platform)
#   model      – downloads + SHA-256-verifies the vendored embedding model (build-time only)
#   builder    – compiles mdkbd (with the onnx embedder) + the client binaries for the
#                *target* platform (built natively under QEMU emulation, so the
#                statically-linked onnxruntime resolves per-arch without cross-link pain)
#   runtime    – minimal Debian base for the daemon image, with the model baked in
#   final      – the mdkbd daemon image shipped to the registry (per --platform)
#   artifacts  – a scratch stage holding just the client binaries, extracted to the host
#                via `docker buildx build --target artifacts --output type=local`
#
# Build deps live in each Rust stage; ca-certificates is needed for cargo and for ort's
# build-time onnxruntime download. The embedding model is vendored into the image at build
# time (see the `model` stage) and loaded from local files at runtime — the daemon never
# downloads a model. ca-certificates is kept only for outbound TLS to a configured remote
# embeddings endpoint (opt-in) and general correctness.
#
# Base must be Debian trixie (not bookworm): ort's prebuilt static onnxruntime references
# newer glibc/libstdc++ symbols (e.g. __isoc23_strtol, __cxa_call_terminate) that bookworm's
# toolchain doesn't provide, so the daemon fails to link there.

ARG ONNX=true


# -- Embedding model (vendored at build time; NO runtime download) ------------
# The int8-quantized BGE-small-en-v1.5 ONNX export (BAAI weights, converted to ONNX by
# Xenova / Joshua Lochner, a Hugging Face maintainer) plus its tokenizer files. Baking these
# in lets the daemon run the neural embedder fully offline. Every file is SHA-256-pinned: if
# any upstream byte changes, the build fails loudly rather than silently shipping a different
# model. ~32 MB on disk; loaded via MDKB_BUNDLED_MODEL_DIR.
#
# MODEL_REPO is overridable (build-arg / the workflow's MODEL_REPO variable) so you can pull
# from a mirror you control instead of depending on the upstream repo; the SHA-256 pins make
# the source interchangeable. The default below is only a fallback for `docker build .`.
FROM debian:trixie-slim AS model
ARG MODEL_REPO=https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /model
RUN set -eux; \
    curl -fsSL "$MODEL_REPO/onnx/model_quantized.onnx" -o model_quantized.onnx; \
    curl -fsSL "$MODEL_REPO/tokenizer.json"            -o tokenizer.json; \
    curl -fsSL "$MODEL_REPO/config.json"               -o config.json; \
    curl -fsSL "$MODEL_REPO/tokenizer_config.json"     -o tokenizer_config.json; \
    curl -fsSL "$MODEL_REPO/special_tokens_map.json"   -o special_tokens_map.json; \
    echo "6c9c6101a956d62dfb5e7190c538226c0c5bb9cb27b651234b6df063ee7dbfe4  model_quantized.onnx"    | sha256sum -c -; \
    echo "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66  tokenizer.json"          | sha256sum -c -; \
    echo "fa73f90bf92c8cace1fbcb709626306f2bdbc9ea3e5b5f94b440df9b6aa56350  config.json"             | sha256sum -c -; \
    echo "9261e7d79b44c8195c1cada2b453e55b00aeb81e907a6664974b4d7776172ab3  tokenizer_config.json"   | sha256sum -c -; \
    echo "b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3  special_tokens_map.json" | sha256sum -c -


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
COPY crates/mdkb-core/Cargo.toml     crates/mdkb-core/
COPY crates/mdkb-index/Cargo.toml    crates/mdkb-index/
COPY crates/mdkb-embed/Cargo.toml    crates/mdkb-embed/
COPY crates/mdkb-protocol/Cargo.toml crates/mdkb-protocol/
COPY crates/mdkb-view/Cargo.toml     crates/mdkb-view/
COPY crates/mdkbd/Cargo.toml         crates/mdkbd/
COPY crates/mdkb-mcp/Cargo.toml      crates/mdkb-mcp/
COPY crates/mdkb-cli/Cargo.toml      crates/mdkb-cli/
COPY crates/mdkb-web/Cargo.toml      crates/mdkb-web/
RUN mkdir -p \
        crates/mdkb-core/src crates/mdkb-index/src crates/mdkb-embed/src \
        crates/mdkb-protocol/src crates/mdkb-view/src \
        crates/mdkbd/src crates/mdkb-mcp/src crates/mdkb-cli/src crates/mdkb-web/src \
    && touch \
        crates/mdkb-core/src/lib.rs crates/mdkb-index/src/lib.rs \
        crates/mdkb-embed/src/lib.rs crates/mdkb-protocol/src/lib.rs \
        crates/mdkb-view/src/lib.rs \
    && for b in mdkbd mdkb-mcp mdkb-cli mdkb-web; do echo 'fn main(){}' > crates/$b/src/main.rs; done \
    && mkdir -p crates/mdkb-embed/examples \
    && echo 'fn main(){}' > crates/mdkb-embed/examples/footprint.rs
RUN cargo fetch

COPY . .
RUN cargo test --workspace

# Docs-as-data drift gate: build the daemon + CLI and verify every generated doc still matches
# its source block in vault/. `mdkb export --check` writes nothing and exits non-zero on drift,
# so a commit that edits a block (or hand-edits a generated file) without re-running export fails
# the build. The CLI is a thin client, so it auto-starts the co-located mdkbd against vault/.
# CI runners can sit on slow/network-backed storage where the daemon's first reconcile (parse all
# blocks + write the index) approaches the default 30s readiness window; grant generous headroom
# so the gate fails only on real drift, never on a cold-start I/O race.
# Pin the machine-local index to a known writable path so the gate doesn't depend on the build
# container's $HOME (the index/socket/lock now live outside the vault by default).
ENV MDKB_READY_TIMEOUT_SECS=180
ENV MDKB_INDEX_DIR=/tmp/mdkb-index
RUN cargo build -p mdkbd -p mdkb-cli \
    && ./target/debug/mdkb export vault --check

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
    && cargo fetch --manifest-path app/mdkb-tauri/src-tauri/Cargo.toml \
    && cargo deny --manifest-path app/mdkb-tauri/src-tauri/Cargo.toml check --config deny.toml


# -- amd64 builder (native on the runner) -------------------------------------
FROM --platform=$BUILDPLATFORM rust:slim-trixie AS builder-amd64
ARG ONNX
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/mdkb-core/Cargo.toml     crates/mdkb-core/
COPY crates/mdkb-index/Cargo.toml    crates/mdkb-index/
COPY crates/mdkb-embed/Cargo.toml    crates/mdkb-embed/
COPY crates/mdkb-protocol/Cargo.toml crates/mdkb-protocol/
COPY crates/mdkb-view/Cargo.toml     crates/mdkb-view/
COPY crates/mdkbd/Cargo.toml         crates/mdkbd/
COPY crates/mdkb-mcp/Cargo.toml      crates/mdkb-mcp/
COPY crates/mdkb-cli/Cargo.toml      crates/mdkb-cli/
COPY crates/mdkb-web/Cargo.toml      crates/mdkb-web/
RUN mkdir -p \
        crates/mdkb-core/src crates/mdkb-index/src crates/mdkb-embed/src \
        crates/mdkb-protocol/src crates/mdkb-view/src \
        crates/mdkbd/src crates/mdkb-mcp/src crates/mdkb-cli/src crates/mdkb-web/src \
    && touch \
        crates/mdkb-core/src/lib.rs crates/mdkb-index/src/lib.rs \
        crates/mdkb-embed/src/lib.rs crates/mdkb-protocol/src/lib.rs \
        crates/mdkb-view/src/lib.rs \
    && for b in mdkbd mdkb-mcp mdkb-cli mdkb-web; do echo 'fn main(){}' > crates/$b/src/main.rs; done \
    && mkdir -p crates/mdkb-embed/examples \
    && echo 'fn main(){}' > crates/mdkb-embed/examples/footprint.rs
RUN cargo fetch

COPY . .
# The daemon ships with semantic embeddings (onnx) by default; the clients are thin and
# never need it. Build them separately so onnx only enters the daemon's dep tree.
RUN if [ "$ONNX" = "true" ]; then \
        cargo build --release -p mdkbd --features onnx ; \
    else \
        cargo build --release -p mdkbd ; \
    fi \
    && cargo build --release -p mdkb-cli -p mdkb-mcp -p mdkb-web
# Normalise output location so final/artifacts stages are arch-agnostic.
RUN mkdir -p /out && cp target/release/mdkbd target/release/mdkb \
        target/release/mdkb-mcp target/release/mdkb-web /out/


# -- arm64 builder (cross-compiled, native speed on the runner) ----------------
# Cross-compilation (not QEMU emulation) keeps the build fast. ort downloads a prebuilt
# aarch64 onnxruntime static lib (it is listed in ort's dist.txt), so onnxruntime is never
# compiled from source here — only linked with the cross toolchain. fastembed uses rustls
# (not openssl), so no libssl / dpkg arm64 cross-libs are needed.
FROM --platform=$BUILDPLATFORM rust:slim-trixie AS builder-arm64
ARG ONNX
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
COPY crates/mdkb-core/Cargo.toml     crates/mdkb-core/
COPY crates/mdkb-index/Cargo.toml    crates/mdkb-index/
COPY crates/mdkb-embed/Cargo.toml    crates/mdkb-embed/
COPY crates/mdkb-protocol/Cargo.toml crates/mdkb-protocol/
COPY crates/mdkb-view/Cargo.toml     crates/mdkb-view/
COPY crates/mdkbd/Cargo.toml         crates/mdkbd/
COPY crates/mdkb-mcp/Cargo.toml      crates/mdkb-mcp/
COPY crates/mdkb-cli/Cargo.toml      crates/mdkb-cli/
COPY crates/mdkb-web/Cargo.toml      crates/mdkb-web/
RUN mkdir -p \
        crates/mdkb-core/src crates/mdkb-index/src crates/mdkb-embed/src \
        crates/mdkb-protocol/src crates/mdkb-view/src \
        crates/mdkbd/src crates/mdkb-mcp/src crates/mdkb-cli/src crates/mdkb-web/src \
    && touch \
        crates/mdkb-core/src/lib.rs crates/mdkb-index/src/lib.rs \
        crates/mdkb-embed/src/lib.rs crates/mdkb-protocol/src/lib.rs \
        crates/mdkb-view/src/lib.rs \
    && for b in mdkbd mdkb-mcp mdkb-cli mdkb-web; do echo 'fn main(){}' > crates/$b/src/main.rs; done \
    && mkdir -p crates/mdkb-embed/examples \
    && echo 'fn main(){}' > crates/mdkb-embed/examples/footprint.rs
RUN cargo fetch --target aarch64-unknown-linux-gnu

COPY . .
RUN if [ "$ONNX" = "true" ]; then \
        cargo build --release --target aarch64-unknown-linux-gnu -p mdkbd --features onnx ; \
    else \
        cargo build --release --target aarch64-unknown-linux-gnu -p mdkbd ; \
    fi \
    && cargo build --release --target aarch64-unknown-linux-gnu -p mdkb-cli -p mdkb-mcp -p mdkb-web
RUN mkdir -p /out && cp \
        target/aarch64-unknown-linux-gnu/release/mdkbd \
        target/aarch64-unknown-linux-gnu/release/mdkb \
        target/aarch64-unknown-linux-gnu/release/mdkb-mcp \
        target/aarch64-unknown-linux-gnu/release/mdkb-web /out/


# -- Runtime base (resolves to the target platform) ---------------------------
FROM debian:trixie-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
# Run as an ARBITRARY uid/gid chosen at deploy time (Kubernetes securityContext, `docker run
# -u`, OpenShift random uids, etc.) — no user is baked in. The image follows the "arbitrary
# user" convention: default to non-root uid 65534 (nobody), make the relevant paths owned by
# the root group (gid 0) and group-writable, and set HOME/dirs so any uid can read the model
# and create its runtime files.
WORKDIR /vault
# Vendored embedding model (baked at build time) — loaded locally, never downloaded.
COPY --from=model /model /opt/mdkb/model
# World-readable model + a group-writable vault dir, so whatever uid the container runs as can
# read the model and write the vault/index. gid 0 (root group) is always a supplemental group.
RUN chgrp -R 0 /opt/mdkb /vault \
    && chmod -R g=u /opt/mdkb /vault
ENV MDKB_BUNDLED_MODEL_DIR=/opt/mdkb/model
ENV HOME=/vault
# Vault is mounted here; the local index is created under $MDKB_VAULT/.mdkb at runtime.
ENV MDKB_VAULT=/vault
EXPOSE 7820
# Default to a non-root uid; the actual uid is overridable at deploy time and the image works
# with any uid (its gid 0 membership grants access to the prepared paths).
USER 65534:0
ENTRYPOINT ["mdkbd"]
# Token must be provided at runtime (e.g. from a Secret) via $MDKB_TOKEN.
CMD ["--vault", "/vault", "--listen", "0.0.0.0:7820"]


# -- Final daemon images (one per architecture) -------------------------------
# Build with `--platform linux/amd64 --target final-amd64` / `linux/arm64 final-arm64`.
# The runtime base inherits the target platform; the heavy compile already happened
# natively in the matching builder stage.
FROM runtime AS final-amd64
COPY --from=builder-amd64 /out/mdkbd /usr/local/bin/mdkbd

FROM runtime AS final-arm64
COPY --from=builder-arm64 /out/mdkbd /usr/local/bin/mdkbd


# -- Release payload, for extraction to the host (not runnable images) --------
# The full daemon + clients + the vendored model, laid out so `mdkbd`/`mdkb` find the model
# automatically via the `<exe_dir>/model` lookup. The container image is just one way to run
# mdkbd; these tarballs are the native way (Linux service / CLI).
FROM scratch AS artifacts-amd64
COPY --from=builder-amd64 /out/mdkbd   /mdkbd
COPY --from=builder-amd64 /out/mdkb     /mdkb
COPY --from=builder-amd64 /out/mdkb-mcp /mdkb-mcp
COPY --from=builder-amd64 /out/mdkb-web /mdkb-web
COPY --from=model         /model        /model

FROM scratch AS artifacts-arm64
COPY --from=builder-arm64 /out/mdkbd   /mdkbd
COPY --from=builder-arm64 /out/mdkb     /mdkb
COPY --from=builder-arm64 /out/mdkb-mcp /mdkb-mcp
COPY --from=builder-arm64 /out/mdkb-web /mdkb-web
COPY --from=model         /model        /model
