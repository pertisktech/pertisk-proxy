#!/bin/sh
# Build release binaries inside docker/Dockerfile.release (BUILDPLATFORM only).
# Host image is Debian bookworm → glibc (gnu) targets for DEB/RPM hosts.
# Always builds via cargo-zigbuild pinned to an older glibc baseline (2.28) so
# the resulting binary runs on distros with older glibc than bookworm's 2.36
# (e.g. AlmaLinux 9 / RHEL9 ship glibc 2.34), regardless of build/target arch match.
set -eu

case "${TARGETARCH}" in
  amd64) RUST_TARGET=x86_64-unknown-linux-gnu.2.28 ;;
  arm64) RUST_TARGET=aarch64-unknown-linux-gnu.2.28 ;;
  *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;;
esac
RUST_TARGET_DIR="${RUST_TARGET%%.*}"

JOBS="${CARGO_BUILD_JOBS:-$(nproc)}"
export CARGO_BUILD_JOBS="${JOBS}"
mkdir -p /app/out

rustup target add "${RUST_TARGET_DIR}"

build_one() {
  bin="$1"
  features="${2:-}"
  if [ -n "${features}" ]; then
    cargo zigbuild --release --locked --target "${RUST_TARGET}" --bin "${bin}" --features "${features}"
  else
    cargo zigbuild --release --locked --target "${RUST_TARGET}" --bin "${bin}"
  fi
  cp "/app/target/${RUST_TARGET_DIR}/release/${bin}" "/app/out/${bin}"
}

case "${PACKAGE_TARGET:-all}" in
  proxy) build_one pertisk-proxy ;;
  ingress) build_one pertisk-proxy-ingress ingress ;;
  *)
    build_one pertisk-proxy
    build_one pertisk-proxy-ingress ingress
    ;;
esac
