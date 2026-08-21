# Prebaked musl builder for CI (apk deps already installed).
# Push once from a networked machine — runners only pull from Harbor.
#
#   ./build/ci-push-builder-image.sh
#
# Used as RUST_IMAGE / BUILDER_IMAGE so Dockerfile RUN never needs Alpine CDN.

ARG RUST_SRC=harbor.tools.pertisk.com/pertisk-proxy/rust:1-alpine3.21
FROM ${RUST_SRC}

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
      file \
      curl \
      musl-dev \
      zig; \
    cargo install cargo-zigbuild --locked
