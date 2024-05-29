ARG IMAGE=rust:bookworm

FROM $IMAGE as build
WORKDIR /app/src
COPY . .
ENV FFMPEG_DIR=/app/ffmpeg
RUN apt update && \
    apt install -y \
    build-essential \
    libx264-dev \
    libwebp-dev \
    libvpx-dev \
    nasm \
    libclang-dev && \
    rm -rf /var/lib/apt/lists/*
RUN git clone --depth=1 https://git.v0l.io/Kieran/FFmpeg.git && \
    cd ffmpeg && \
    ./configure \
    --prefix=$FFMPEG_DIR \
    --disable-programs \
    --disable-doc \
    --disable-network \
    --enable-gpl \
    --enable-version3 \
    --enable-libx264 \
    --enable-libwebp \
    --enable-libvpx \
    --disable-static \
    --enable-shared && \
    make -j8 && make install
RUN cargo install --path . --root /app/build

FROM $IMAGE as runner
WORKDIR /app
RUN apt update && \
    apt install -y libx264-164 libwebp7 libvpx7 && \
    rm -rf /var/lib/apt/lists/*
COPY --from=build /app/build .
COPY --from=build /app/src/ui ui
COPY --from=build /app/ffmpeg/lib/ /lib
ENTRYPOINT ["./bin/void_cat"]