ARG IMAGE=rust:trixie
ARG FEATURES

FROM voidic/rust-ffmpeg AS build
WORKDIR /src
COPY . .
RUN apt update && \
    apt install -y protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*
RUN cargo install --path . --root /app/build --features "${FEATURES}"

FROM node:trixie AS ui_builder
WORKDIR /app/src
COPY ui_src .
RUN yarn && yarn build

FROM $IMAGE AS runner
LABEL org.opencontainers.image.source="https://git.v0l.io/Kieran/route96"
LABEL org.opencontainers.image.licenses="MIT"
LABEL org.opencontainers.image.authors="Kieran"
WORKDIR /app
RUN apt update && \
    apt install -y \
    libx264-164 \
    libx265-215 \
    libvpx9 \
    libopus0 \
    libwebp7 \
    libdav1d7 \
    va-driver-all \
    libva-drm2 \
    libva-x11-2 \
    libva-wayland2 \
    libva-glx2 && \
    if [ "$(dpkg --print-architecture)" == "amd64" ]; then \
        apt install -y libvpl-dev; \
    fi \
     && \
    rm -rf /var/lib/apt/lists/*
COPY --from=build /app/build .
COPY --from=ui_builder /app/src/dist ui
COPY --from=build /app/src/ffmpeg/lib/ /lib
RUN ./bin/route96 --version
ENTRYPOINT ["./bin/route96"]