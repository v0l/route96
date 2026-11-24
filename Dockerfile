ARG FEATURES="blossom,nip96,react-ui,r96util,media-compression,labels"

# ── Rust dependency cache ─────────────────────────────────────────────────────
# Install build tools and pre-compile all dependencies in isolation.
# This layer is only invalidated when Cargo.toml, Cargo.lock, or the base
# image change — not on every source edit.
FROM voidic/rust-ffmpeg AS rust-deps
WORKDIR /src
RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY media-worker/Cargo.toml ./media-worker/Cargo.toml
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/bin/main.rs && \
    echo "fn main() {}" > src/bin/r96util.rs && \
    echo "" > src/lib.rs && \
    mkdir -p media-worker/src src/media && \
    echo "fn main() {}" > media-worker/src/main.rs && \
    touch src/media/protocol.rs
ARG FEATURES
RUN cargo build --release --no-default-features --features "${FEATURES}" --workspace && \
    # Remove stub artifacts so the real source build re-compiles only the app.
    rm -f target/release/route96 \
          target/release/deps/route96-* \
          target/release/route96-media \
          target/release/deps/route96_media-* \
          target/release/deps/libroute96-*

# ── Rust application build ────────────────────────────────────────────────────
FROM rust-deps AS rust-build
COPY src ./src
COPY media-worker ./media-worker
COPY migrations ./migrations
COPY docs ./docs
# Touch entry points so Cargo sees them as changed vs. the stubs above.
RUN touch src/lib.rs src/bin/main.rs src/bin/r96util.rs media-worker/src/main.rs
ARG FEATURES
RUN cargo build --release --no-default-features --features "${FEATURES}" --workspace && \
    mkdir -p /app/bin && \
    cp target/release/route96 /app/bin/route96 && \
    cp target/release/route96-media /app/bin/route96-media

# ── UI build ──────────────────────────────────────────────────────────────────
FROM node:22 AS ui-build
RUN npm install -g corepack@latest --force && corepack enable
WORKDIR /app/src
COPY ui_src .
RUN yarn install --immutable && yarn build

# ── Runtime image ─────────────────────────────────────────────────────────────
FROM debian:trixie-slim
LABEL org.opencontainers.image.source="https://github.com/v0l/route96"
LABEL org.opencontainers.image.licenses="MIT"
LABEL org.opencontainers.image.authors="Kieran"
WORKDIR /app

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        libx264-164 \
        libx265-215 \
        libvpx9 \
        libopus0 \
        libwebp7 \
        libwebpmux3 \
        libdav1d7 \
        va-driver-all \
        libva-drm2 \
        libva-x11-2 \
        libva-wayland2 \
        libva-glx2 && \
    if [ "$(dpkg --print-architecture)" = "amd64" ]; then \
        apt-get install -y --no-install-recommends libvpl2; \
    fi && \
    rm -rf /var/lib/apt/lists/*

COPY --from=rust-build /app/bin          ./bin
COPY --from=rust-build /app/src/ffmpeg/lib/  /lib
COPY --from=ui-build   /app/src/dist     ./ui
COPY entrypoint.sh                       ./entrypoint.sh

ENV RUST_BACKTRACE=1

RUN groupadd --gid 10001 route96 && \
    useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin route96 && \
    mkdir -p /data /app/data /tmp && chown route96:route96 /data /app/data
USER 10001:10001

RUN ./bin/route96 --version
ENTRYPOINT ["./entrypoint.sh"]
