# syntax=docker/dockerfile:1

# Reference image for the Drishti HTTP service. It ships the starter config, so
# `docker run` starts the server and downloads the models on first use into a
# cache volume. Multi-arch (linux/amd64, linux/arm64) is produced in CI.

# ---- builder ----
# The server is built with ort's load-dynamic feature, so the build links no
# ONNX Runtime and needs only a Rust toolchain (pure Rust, any glibc).
FROM rust:1-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p drishti-server

# ---- runtime ----
FROM debian:bookworm-slim AS runtime
ARG TARGETARCH
# Pin the ONNX Runtime version to the C API (v24) this ort build targets.
ARG ORT_VERSION=1.24.2
# Fetch Microsoft's official ONNX Runtime release (broadly glibc-compatible) and
# place its shared library where ort will dlopen it via ORT_DYLIB_PATH. This is
# how the server gets ONNX Runtime without linking it at build time.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgomp1 curl \
    && case "$TARGETARCH" in \
         amd64) ORT_ARCH=x64 ;; \
         arm64) ORT_ARCH=aarch64 ;; \
         *) echo "unsupported TARGETARCH: $TARGETARCH" && exit 1 ;; \
       esac \
    && curl -fsSL "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}.tgz" -o /tmp/ort.tgz \
    && tar -xzf /tmp/ort.tgz -C /tmp \
    && cp -P /tmp/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}/lib/libonnxruntime.so* /usr/local/lib/ \
    && rm -rf /tmp/ort.tgz /tmp/onnxruntime-linux-* \
    && apt-get purge -y curl && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/drishti-server /usr/local/bin/drishti-server
COPY config/starter.toml /etc/drishti/config.toml

ENV LD_LIBRARY_PATH=/usr/local/lib
ENV ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
# Models download here on first run; mount a volume to persist them across restarts.
ENV XDG_CACHE_HOME=/var/cache
VOLUME ["/var/cache/drishti"]

EXPOSE 8080
ENTRYPOINT ["drishti-server", "--config", "/etc/drishti/config.toml", "--bind", "0.0.0.0:8080"]
