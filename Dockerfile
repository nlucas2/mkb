# mdkb daemon image.
#
# Multi-stage: build the workspace binaries, then ship a slim runtime with just the daemon.
# The daemon serves over TCP (token-gated) for in-cluster clients; the vault is a mounted
# volume and the local index lives under <vault>/.mdkb on the pod's own storage.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
# Build with the local ONNX embedder so semantic search works server-side in the pod.
RUN cargo build --release -p mdkbd --features onnx

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/mdkbd /usr/local/bin/mdkbd

# Vault is mounted here; the local index is created under $MDKB_VAULT/.mdkb at runtime.
ENV MDKB_VAULT=/vault
VOLUME ["/vault"]
EXPOSE 7820

# Token must be provided at runtime (e.g. from a Secret) via $MDKB_TOKEN.
ENTRYPOINT ["mdkbd"]
CMD ["--vault", "/vault", "--listen", "0.0.0.0:7820"]
