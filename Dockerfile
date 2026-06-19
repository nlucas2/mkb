# syntax=docker/dockerfile:1
#
# Multi-stage build for mdkb.
#
# Stages:
#   tester     – runs `cargo test --workspace` (native, on the build platform)
#   builder    – compiles mdkbd (with the onnx embedder) + the client binaries for the
#                *target* platform (built natively under QEMU emulation, so the
#                statically-linked onnxruntime resolves per-arch without cross-link pain)
#   runtime    – minimal Debian base for the daemon image
#   final      – the mdkbd daemon image shipped to the registry (per --platform)
#   artifacts  – a scratch stage holding just the client binaries, extracted to the host
#                via `docker buildx build --target artifacts --output type=local`
#
# Build deps live in each Rust stage; ca-certificates is needed for cargo and for ort's
# build-time onnxruntime download. The daemon image keeps ca-certificates so the embedding
# model can be fetched at first use (it degrades to the offline hash embedder if the
# network is unavailable).
#
# Base must be Debian trixie (not bookworm): ort's prebuilt static onnxruntime references
# newer glibc/libstdc++ symbols (e.g. __isoc23_strtol, __cxa_call_terminate) that bookworm's
# toolchain doesn't provide, so the daemon fails to link there.

ARG ONNX=true


# -- Test stage ---------------------------------------------------------------
FROM --platform=$BUILDPLATFORM rust:slim-trixie AS tester
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config ca-certificates \
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
    && for b in mdkbd mdkb-mcp mdkb-cli mdkb-web; do echo 'fn main(){}' > crates/$b/src/main.rs; done
RUN cargo fetch

COPY . .
RUN cargo test --workspace


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
    && for b in mdkbd mdkb-mcp mdkb-cli mdkb-web; do echo 'fn main(){}' > crates/$b/src/main.rs; done
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
    && for b in mdkbd mdkb-mcp mdkb-cli mdkb-web; do echo 'fn main(){}' > crates/$b/src/main.rs; done
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
RUN useradd --system --uid 10001 --home-dir /vault mdkb
WORKDIR /vault
# Vault is mounted here; the local index is created under $MDKB_VAULT/.mdkb at runtime.
ENV MDKB_VAULT=/vault
EXPOSE 7820
USER mdkb
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


# -- Client binaries, for extraction to the host (not runnable images) --------
FROM scratch AS artifacts-amd64
COPY --from=builder-amd64 /out/mdkb     /mdkb
COPY --from=builder-amd64 /out/mdkb-mcp /mdkb-mcp
COPY --from=builder-amd64 /out/mdkb-web /mdkb-web

FROM scratch AS artifacts-arm64
COPY --from=builder-arm64 /out/mdkb     /mdkb
COPY --from=builder-arm64 /out/mdkb-mcp /mdkb-mcp
COPY --from=builder-arm64 /out/mdkb-web /mdkb-web
