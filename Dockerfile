# Multi-stage build: one stage compiles the web app (WASM) and the server,
# the final stage holds only the server binary and the static files.
#
#   docker build -t qiyiqdoppi .
#
# The cache mounts keep the cargo registry and build artefacts between
# builds, so rebuilding after a small change takes a minute or two.

ARG RUST_VERSION=1.97.0
# Where the base images come from. If Docker Hub rate-limits you, build with
#   --build-arg BASE_REGISTRY=public.ecr.aws/docker/library
ARG BASE_REGISTRY=docker.io/library

FROM ${BASE_REGISTRY}/rust:${RUST_VERSION}-bookworm AS build
RUN rustup target add wasm32-unknown-unknown
# Pinned build tools (same versions as CI and docs/VERSIONS.md).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install trunk --version 0.22.1 --locked && \
    cargo install wasm-bindgen-cli --version 0.2.130 --locked
WORKDIR /src
COPY . .
# Compile-time checked SQL uses the committed metadata in .sqlx/, so no
# database is needed to build.
ENV SQLX_OFFLINE=true
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    scripts/build-web.sh && \
    cargo build --release --locked -p server && \
    mkdir -p /out && \
    cp target/release/qiyiq-server /out/ && \
    cp -r crates/web/dist /out/web

FROM ${BASE_REGISTRY}/debian:bookworm-slim
# CA certificates for outbound HTTPS (object storage, Telegram, try-on API).
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /app qiyiq \
    && mkdir -p /data/storage && chown -R qiyiq /data
WORKDIR /app
COPY --from=build /out/qiyiq-server /usr/local/bin/qiyiq-server
COPY --from=build /out/web /app/web
# Run as an unprivileged user.
USER qiyiq
ENV BIND_ADDR=0.0.0.0:8080 \
    STATIC_DIR=/app/web \
    STORAGE_FS_DIR=/data/storage
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=5 CMD ["qiyiq-server", "healthcheck"]
ENTRYPOINT ["qiyiq-server"]
