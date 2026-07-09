# syntax=docker/dockerfile:1.7

ARG TARGETPLATFORM

FROM node:22-alpine AS admin
WORKDIR /admin
COPY admin/package.json admin/pnpm-lock.yaml ./
RUN corepack enable
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile
COPY admin/ ./
RUN pnpm build

FROM rust:1-alpine AS chef
RUN apk add --no-cache \
    build-base \
    pkgconfig \
    openssl-dev \
    perl \
    cmake \
    clang \
    clang-dev \
    go \
    nasm \
    musl-dev
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG TARGETPLATFORM
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked,id=pertisk-ingress-registry-${TARGETPLATFORM} \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked,id=pertisk-ingress-git-${TARGETPLATFORM} \
    --mount=type=cache,target=/app/target,sharing=locked,id=pertisk-ingress-target-${TARGETPLATFORM} \
    cargo chef cook --release --locked --recipe-path recipe.json \
    --no-default-features --features ingress,acme,h3-quinn,prometheus

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY --from=admin /admin/dist ./admin/dist
ENV RUST_MIN_STACK=16777216
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked,id=pertisk-ingress-registry-${TARGETPLATFORM} \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked,id=pertisk-ingress-git-${TARGETPLATFORM} \
    --mount=type=cache,target=/app/target,sharing=locked,id=pertisk-ingress-target-${TARGETPLATFORM} \
    cargo build --release --locked --no-default-features \
    --features ingress,acme,h3-quinn,prometheus --bin pertisk-proxy-ingress \
    && install -D /app/target/release/pertisk-proxy-ingress /usr/local/bin/pertisk-proxy-ingress

FROM alpine:3.21
RUN apk add --no-cache ca-certificates openssl
COPY --from=builder /usr/local/bin/pertisk-proxy-ingress /usr/local/bin/pertisk-proxy-ingress
COPY --from=admin /admin/dist /usr/share/pertisk-proxy/admin/dist
USER 65532:65532
EXPOSE 8080 8443 8443/udp 9080
ENTRYPOINT ["/usr/local/bin/pertisk-proxy-ingress"]
