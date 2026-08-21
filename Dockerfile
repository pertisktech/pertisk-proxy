# syntax=docker/dockerfile:1.7

# Prefer docker/Dockerfile.ingress for scratch runtime; this root Dockerfile matches
# local: docker buildx build -f Dockerfile --platform linux/amd64,linux/arm64 --push
#
# Admin UI must be prebuilt: make admin-dist (no node: pull).
# Builder default: Harbor rust mirror. Final alpine still needs network or a Harbor
# alpine mirror (./build/ci-mirror-base-images.sh). Release CI does NOT use this
# file — it assembles glibc images via build/ci-docker-images-from-bins.sh.

ARG RUST_IMAGE=harbor.tools.pertisk.com/pertisk-proxy/rust:1-alpine3.21
# Local/dev fallback when Harbor alpine mirror is missing (ECR rate limits).
ARG ALPINE_IMAGE=public.ecr.aws/docker/library/alpine:3.21

FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS builder
ARG TARGETPLATFORM
ARG TARGETARCH
ARG BUILDARCH
COPY docker/alpine-apk-setup.sh /usr/local/sbin/alpine-apk-setup.sh
RUN set -eux; \
    . /usr/local/sbin/alpine-apk-setup.sh; \
    apk_add_retry \
      build-base \
      pkgconf \
      openssl-dev \
      perl \
      cmake \
      clang \
      clang-dev \
      go \
      nasm \
      musl-dev; \
    if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then \
      apk_add_retry zig; \
      cargo install cargo-zigbuild --locked; \
    fi
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
COPY admin/dist ./admin/dist
ENV RUST_MIN_STACK=16777216
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked,id=pertisk-ingress-registry-${TARGETPLATFORM} \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked,id=pertisk-ingress-git-${TARGETPLATFORM} \
    --mount=type=cache,target=/app/target,sharing=locked,id=pertisk-ingress-target-${TARGETPLATFORM} \
    test -f admin/dist/index.html \
    && case "${TARGETARCH}" in \
      amd64) RUST_TARGET=x86_64-unknown-linux-musl ;; \
      arm64) RUST_TARGET=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && rustup target add "${RUST_TARGET}" \
    && if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then \
         cargo zigbuild --release --locked --target "${RUST_TARGET}" \
           --bin pertisk-proxy-ingress --features ingress; \
       else \
         cargo build --release --locked --target "${RUST_TARGET}" \
           --bin pertisk-proxy-ingress --features ingress; \
       fi \
    && install -D "/app/target/${RUST_TARGET}/release/pertisk-proxy-ingress" /usr/local/bin/pertisk-proxy-ingress

FROM ${ALPINE_IMAGE}
COPY docker/alpine-apk-setup.sh /usr/local/sbin/alpine-apk-setup.sh
RUN set -eux; \
    . /usr/local/sbin/alpine-apk-setup.sh; \
    apk_add_retry ca-certificates openssl
COPY --from=builder /usr/local/bin/pertisk-proxy-ingress /usr/local/bin/pertisk-proxy-ingress
COPY admin/dist /usr/share/pertisk-proxy/admin/dist
USER 65532:65532
EXPOSE 8080 8443 8443/udp 9080
ENTRYPOINT ["/usr/local/bin/pertisk-proxy-ingress"]
