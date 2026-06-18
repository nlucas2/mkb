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


# -- Builder (target platform; native under emulation) ------------------------
FROM rust:slim-trixie AS builder
ARG ONNX
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config ca-certificates libssl-dev \
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
    fi
RUN cargo build --release -p mdkb-cli -p mdkb-mcp -p mdkb-web


# -- Runtime base -------------------------------------------------------------
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


# -- Final daemon image (one per architecture, selected by --platform) --------
FROM runtime AS final
COPY --from=builder /app/target/release/mdkbd /usr/local/bin/mdkbd


# -- Client binaries, for extraction to the host (not a runnable image) -------
FROM scratch AS artifacts
COPY --from=builder /app/target/release/mdkb     /mdkb
COPY --from=builder /app/target/release/mdkb-mcp /mdkb-mcp
COPY --from=builder /app/target/release/mdkb-web /mdkb-web
