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
    && for b in mdkbd mdkb-mcp mdkb-cli mdkb-web; do echo 'fn main(){}' > crates/$b/src/main.rs; done \
    && mkdir -p crates/mdkb-embed/examples \
    && echo 'fn main(){}' > crates/mdkb-embed/examples/footprint.rs
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
RUN useradd --system --uid 10001 --home-dir /vault mdkb
WORKDIR /vault
# Vendored embedding model (baked at build time) — loaded locally, never downloaded.
COPY --from=model /model /opt/mdkb/model
ENV MDKB_BUNDLED_MODEL_DIR=/opt/mdkb/model
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
