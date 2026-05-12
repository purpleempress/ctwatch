# syntax=docker/dockerfile:1.7

FROM rust:1.82-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*

ENV SQLX_OFFLINE=true

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .sqlx ./.sqlx
COPY src ./src
COPY migrations ./migrations
COPY public_suffix_list.dat ./
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --no-create-home --shell /usr/sbin/nologin ctwatch

COPY --from=builder /build/target/release/ctwatch /usr/local/bin/ctwatch

USER ctwatch
EXPOSE 8080
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/ctwatch"]
CMD ["serve"]
