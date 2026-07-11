FROM rust:1.78-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --features bedrock 2>/dev/null || cargo build --release

COPY . .
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates git && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/rem /usr/local/bin/rem

ENV RUST_BACKTRACE=1

ENTRYPOINT ["rem"]
CMD ["--help"]
