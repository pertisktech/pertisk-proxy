# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --bin pertisk-proxy-ingress --features ingress

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/pertisk-proxy-ingress /usr/local/bin/pertisk-proxy-ingress
USER 65532:65532
EXPOSE 8080 8443 8443/udp
ENTRYPOINT ["/usr/local/bin/pertisk-proxy-ingress"]
