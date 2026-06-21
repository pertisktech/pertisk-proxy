# syntax=docker/dockerfile:1

FROM node:22-bookworm AS admin
WORKDIR /admin
COPY admin/package.json admin/pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile
COPY admin/ ./
RUN pnpm build

FROM rust:1-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev perl cmake clang libclang-dev golang-go nasm \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY --from=admin /admin/dist ./admin/dist
ENV RUST_MIN_STACK=16777216
RUN cargo build --release --locked --bin pertisk-proxy-ingress --features ingress

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/pertisk-proxy-ingress /usr/local/bin/pertisk-proxy-ingress
COPY --from=admin /admin/dist /usr/share/pertisk-proxy/admin/dist
USER 65532:65532
EXPOSE 8080 8443 8443/udp 9080
ENTRYPOINT ["/usr/local/bin/pertisk-proxy-ingress"]
