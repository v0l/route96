ARG IMAGE=rust:bookworm
ARG FEATURES

FROM $IMAGE AS build
WORKDIR /app/src
COPY src src
COPY migrations migrations
COPY Cargo.lock Cargo.lock
COPY Cargo.toml Cargo.toml
ENV FFMPEG_DIR=/app/ffmpeg
RUN apt update && \
    apt install -y \
    build-essential \
    libx264-dev \
    libwebp-dev \
    libvpx-dev \
    nasm \
    libclang-dev \
    protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*
RUN git clone --single-branch --branch release/7.1 https://git.v0l.io/ffmpeg/FFmpeg.git && \
    cd FFmpeg && \
    ./configure \
    --prefix=${FFMPEG_DIR} \
    --disable-programs \
    --disable-doc \
    --disable-network \
    --enable-gpl \
    --enable-libx264 \
    --enable-libwebp \
    --enable-libvpx \
    --disable-static \
    --disable-postproc \
    --enable-shared && \
    make -j$(nproc) install
RUN cargo install --path . --root /app/build --features "${FEATURES}"

FROM node:bookworm AS ui_builder
WORKDIR /app/src
COPY ui_src .
RUN yarn && yarn build

FROM $IMAGE AS runner
LABEL org.opencontainers.image.source="https://git.v0l.io/Kieran/route96"
LABEL org.opencontainers.image.licenses="MIT"
LABEL org.opencontainers.image.authors="Kieran"
WORKDIR /app
RUN apt update && \
    apt install -y libx264-164 libwebp7 libvpx7 && \
    rm -rf /var/lib/apt/lists/*
COPY --from=build /app/build .
COPY --from=ui_builder /app/src/dist ui
COPY --from=build /app/ffmpeg/lib/ /lib
RUN ./bin/route96 --version
ENTRYPOINT ["./bin/route96"]