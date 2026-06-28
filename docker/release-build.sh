#!/bin/sh
# Build release binaries inside docker/Dockerfile.release (BUILDPLATFORM only).
set -eu

case "${TARGETARCH}" in
  amd64) RUST_TARGET=x86_64-unknown-linux-musl ;;
  arm64) RUST_TARGET=aarch64-unknown-linux-musl ;;
  *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;;
esac

JOBS="${CARGO_BUILD_JOBS:-$(nproc)}"
export CARGO_BUILD_JOBS="${JOBS}"
mkdir -p /app/out

if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then
  rustup target add "${RUST_TARGET}"
fi

build_one() {
  bin="$1"
  features="${2:-}"
  if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then
    if [ -n "${features}" ]; then
      cargo zigbuild --release --locked --target "${RUST_TARGET}" --bin "${bin}" --features "${features}"
    else
      cargo zigbuild --release --locked --target "${RUST_TARGET}" --bin "${bin}"
    fi
    dir="/app/target/${RUST_TARGET}/release"
  else
    if [ -n "${features}" ]; then
      cargo build --release --locked --bin "${bin}" --features "${features}"
    else
      cargo build --release --locked --bin "${bin}"
    fi
    dir="/app/target/release"
  fi
  cp "${dir}/${bin}" "/app/out/${bin}"
}

case "${PACKAGE_TARGET:-all}" in
  proxy) build_one pertisk-proxy ;;
  ingress) build_one pertisk-proxy-ingress ingress ;;
  *)
    build_one pertisk-proxy
    build_one pertisk-proxy-ingress ingress
    ;;
esac
