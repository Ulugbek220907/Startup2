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
# Pinned build tools (same versions as CI and docs/VERSIONS.md), downloaded
# as release binaries and checked against known SHA-256 sums. Compiling
# them from source would add many minutes on a small build machine.
ARG TRUNK_VERSION=0.22.1
ARG WASM_BINDGEN_VERSION=0.2.130
RUN set -eu; \
    case "$(uname -m)" in \
      x86_64) trunk_arch=x86_64-unknown-linux-gnu; trunk_sum=2ab8d548fd5ed9cda50906f61fd8654204e7893bd52ebbfd90533862b7d71ee1; \
              wb_arch=x86_64-unknown-linux-musl; wb_sum=9687f3bb551b9f66f37ed32a40d7913a00f7962db653054e56857bc306c3227f ;; \
      aarch64) trunk_arch=aarch64-unknown-linux-gnu; trunk_sum=b4017d1e0bfcffb4f4bee2be37c48def21e6e6896c3d0f503c43c42f714e57ea; \
               wb_arch=aarch64-unknown-linux-gnu; wb_sum=cc8dd8e90aa9111758168d30c63a14b7f0276997d9d60d3e160a1fb5e7d3b919 ;; \
      *) echo "unsupported build architecture $(uname -m)" >&2; exit 1 ;; \
    esac; \
    cd /tmp; \
    curl -fsSL --retry 3 -o trunk.tgz "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-${trunk_arch}.tar.gz"; \
    echo "${trunk_sum}  trunk.tgz" | sha256sum -c -; \
    tar -xzf trunk.tgz -C /usr/local/bin trunk; \
    curl -fsSL --retry 3 -o wb.tgz "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WASM_BINDGEN_VERSION}/wasm-bindgen-${WASM_BINDGEN_VERSION}-${wb_arch}.tar.gz"; \
    echo "${wb_sum}  wb.tgz" | sha256sum -c -; \
    tar -xzf wb.tgz --strip-components=1 -C /usr/local/bin "wasm-bindgen-${WASM_BINDGEN_VERSION}-${wb_arch}/wasm-bindgen"; \
    rm -f trunk.tgz wb.tgz; \
    trunk --version; wasm-bindgen --version
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
# Run as an unprivileged user. Hosts that mount persistent disks owned by
# root (see docs/RENDER.md) can build with --build-arg RUN_AS=root.
ARG RUN_AS=qiyiq
USER ${RUN_AS}
# The port comes from BIND_ADDR, else PORT, else 8080.
ENV STATIC_DIR=/app/web \
    STORAGE_FS_DIR=/data/storage
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=5 CMD ["qiyiq-server", "healthcheck"]
ENTRYPOINT ["qiyiq-server"]
