# EOF — one binary with the web UI and DB migrations compiled in.

FROM rust:1-slim-bookworm AS build
WORKDIR /src
# Cache dependencies separately from app code.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src target/release/deps/eof*
COPY src ./src
COPY migrations ./migrations
COPY web ./web
RUN cargo build --release

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/eof /eof
ENV PORT=8080
EXPOSE 8080
USER nonroot
ENTRYPOINT ["/eof"]
