# EOF — one binary with the web UI and DB migrations compiled in.

FROM rust:1-slim-bookworm AS build
WORKDIR /src
# Cache dependencies separately from app code.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src target/release/deps/eof*
COPY src ./src
COPY migrations ./migrations
COPY web ./web
# COPY keeps the files' original (older) mtimes, so without the touch Cargo may treat the
# placeholder binary from the cache step as fresh and ship an empty `fn main() {}`.
RUN touch src/main.rs && cargo build --release && test "$(stat -c%s target/release/eof)" -gt 1000000

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/eof /eof
ENV PORT=8080
EXPOSE 8080
USER nonroot
ENTRYPOINT ["/eof"]
