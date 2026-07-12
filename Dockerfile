# syntax=docker/dockerfile:1

# Reference image for the Drishti HTTP service. It ships the starter config, so
# `docker run` starts the server and downloads the models on first use into a
# cache volume. Multi-arch (linux/amd64, linux/arm64) is produced in CI.

# ---- builder ----
FROM rust:1-bookworm AS builder
WORKDIR /build
COPY . .
# Build only the server binary. ort downloads the ONNX Runtime shared library at
# build time and places it alongside the binary in target/release.
RUN cargo build --release -p drishti-server

# ---- runtime ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgomp1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/drishti-server /usr/local/bin/drishti-server
COPY --from=builder /build/target/release/libonnxruntime*.so* /usr/local/lib/
COPY config/starter.toml /etc/drishti/config.toml

ENV LD_LIBRARY_PATH=/usr/local/lib
# Models download here on first run; mount a volume to persist them across restarts.
ENV XDG_CACHE_HOME=/var/cache
VOLUME ["/var/cache/drishti"]

EXPOSE 8080
ENTRYPOINT ["drishti-server", "--config", "/etc/drishti/config.toml", "--bind", "0.0.0.0:8080"]
