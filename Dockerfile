# syntax=docker/dockerfile:1.7

# Local (networked):
#   make admin-dist
#   docker buildx build --platform linux/amd64,linux/arm64 --provenance=false \
#     -f Dockerfile -t …/ingress:VERSION --push .
#
# CI (Harbor-only; no docker.io / apk CDN):
#   --build-arg RUST_IMAGE=harbor…/builder:alpine-rust
#   --build-arg ALPINE_IMAGE=harbor…/runtime:alpine
#   --build-arg SKIP_BUILD_DEPS=1
#   --build-arg SKIP_RUNTIME_APK=1

ARG RUST_IMAGE=public.ecr.aws/docker/library/rust:1-alpine3.21
ARG ALPINE_IMAGE=public.ecr.aws/docker/library/alpine:3.21
ARG SKIP_BUILD_DEPS=0
ARG SKIP_RUNTIME_APK=0

FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS builder
ARG TARGETPLATFORM
ARG TARGETARCH
ARG BUILDARCH
ARG SKIP_BUILD_DEPS=0
COPY docker/alpine-apk-setup.sh /usr/local/sbin/alpine-apk-setup.sh
RUN set -eux; \
    if [ "${SKIP_BUILD_DEPS}" = "1" ]; then \
      echo "SKIP_BUILD_DEPS=1 (using prebaked builder image)"; \
      command -v cargo >/dev/null; \
      command -v zig >/dev/null || true; \
    else \
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
      fi; \
    fi
WORKDIR /app

COPY Cargo.toml Cargo.lock build.rs ./
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
ARG SKIP_RUNTIME_APK=0
COPY docker/alpine-apk-setup.sh /usr/local/sbin/alpine-apk-setup.sh
RUN set -eux; \
    if [ "${SKIP_RUNTIME_APK}" = "1" ]; then \
      echo "SKIP_RUNTIME_APK=1 (using prebaked alpine runtime)"; \
      test -f /etc/ssl/certs/ca-certificates.crt; \
    else \
      . /usr/local/sbin/alpine-apk-setup.sh; \
      apk_add_retry ca-certificates openssl; \
    fi
COPY --from=builder /usr/local/bin/pertisk-proxy-ingress /usr/local/bin/pertisk-proxy-ingress
COPY admin/dist /usr/share/pertisk-proxy/admin/dist
USER 65532:65532
EXPOSE 8080 8443 8443/udp 9080
ENTRYPOINT ["/usr/local/bin/pertisk-proxy-ingress"]
