ARG IMAGE=rust:bookworm

FROM $IMAGE as build
WORKDIR /app/src
COPY . .
RUN cargo install --path . --root /app/build

FROM $IMAGE as runner
WORKDIR /app
COPY --from=build /app/build .
COPY --from=build /app/src/ui ui
ENTRYPOINT ["/app/bin/void_cat"]