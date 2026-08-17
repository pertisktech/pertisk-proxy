# syntax=docker/dockerfile:1.7

# All RUN steps execute on BUILDPLATFORM (native speed, no QEMU emulation).
# Cross-compilation to the target arch uses cargo-zigbuild (arm64<->amd64).
# BUILDPLATFORM/TARGETPLATFORM/TARGETARCH/BUILDARCH are auto-provided by buildx;
# do NOT redeclare them globally (shadows the built-in values with empty strings).

FROM --platform=$BUILDPLATFORM node:22-alpine AS admin
WORKDIR /admin
COPY admin/package.json admin/pnpm-lock.yaml ./
RUN corepack enable
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile
COPY admin/ ./
RUN pnpm build

FROM --platform=$BUILDPLATFORM rust:1-alpine AS builder
ARG TARGETPLATFORM
ARG TARGETARCH
ARG BUILDARCH
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
    musl-dev \
    zig \
    && cargo install cargo-zigbuild --locked
WORKDIR /app

COPY Cargo.toml Cargo.lock build.rs ./
# Workspace members must exist for cargo fetch/build (image does not compile them here).
COPY tunnel/proto/Cargo.toml tunnel/proto/
COPY tunnel/server/Cargo.toml tunnel/server/
COPY tunnel/client/Cargo.toml tunnel/client/
RUN mkdir -p tunnel/proto/src tunnel/server/src tunnel/client/src \
    && touch tunnel/proto/src/lib.rs tunnel/server/src/main.rs tunnel/client/src/main.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked,id=pertisk-ingress-registry-${TARGETPLATFORM} \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked,id=pertisk-ingress-git-${TARGETPLATFORM} \
    cargo fetch --locked

COPY src ./src
COPY tunnel ./tunnel
COPY --from=admin /admin/dist ./admin/dist
ENV RUST_MIN_STACK=16777216
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked,id=pertisk-ingress-registry-${TARGETPLATFORM} \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked,id=pertisk-ingress-git-${TARGETPLATFORM} \
    --mount=type=cache,target=/app/target,sharing=locked,id=pertisk-ingress-target-${TARGETPLATFORM} \
    case "${TARGETARCH}" in \
      amd64) RUST_TARGET=x86_64-unknown-linux-musl ;; \
      arm64) RUST_TARGET=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && rustup target add "${RUST_TARGET}" \
    && if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then \
         cargo zigbuild --release --locked --target "${RUST_TARGET}" \
           --no-default-features --features ingress,acme,h3-quinn,prometheus \
           --bin pertisk-proxy-ingress; \
       else \
         cargo build --release --locked --target "${RUST_TARGET}" \
           --no-default-features --features ingress,acme,h3-quinn,prometheus \
           --bin pertisk-proxy-ingress; \
       fi \
    && install -D "/app/target/${RUST_TARGET}/release/pertisk-proxy-ingress" /usr/local/bin/pertisk-proxy-ingress

FROM alpine:3.21
RUN apk add --no-cache ca-certificates openssl
COPY --from=builder /usr/local/bin/pertisk-proxy-ingress /usr/local/bin/pertisk-proxy-ingress
COPY --from=admin /admin/dist /usr/share/pertisk-proxy/admin/dist
USER 65532:65532
EXPOSE 8080 8443 8443/udp 9080
ENTRYPOINT ["/usr/local/bin/pertisk-proxy-ingress"]
