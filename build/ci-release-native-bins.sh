#!/usr/bin/env bash
# Build release Linux binaries on the runner host (no Docker apt).
# Usage: ./build/ci-release-native-bins.sh <amd64|arm64> <VERSION> <proxy|ingress|all>
#
# Tuned for multi-core CI hosts (e.g. 16C/32GB): one cargo invocation for
# proxy+ingress so dependency crates compile once.
set -euo pipefail

ARCH="${1:?arch}"
VERSION="${2:?version}"
TARGET="${3:-all}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

case "$(uname -m)" in
  x86_64|amd64) HOST=amd64 ;;
  aarch64|arm64) HOST=arm64 ;;
  *) echo "unsupported host arch: $(uname -m)" >&2; exit 1 ;;
esac
if [ "$ARCH" != "$HOST" ]; then
  echo "native build only supports host arch ($HOST), got $ARCH" >&2
  exit 1
fi

chmod +x build/ci-install-deps.sh
./build/ci-install-deps.sh

# Pin the glibc baseline via zig so DEB/RPM binaries run on older-glibc distros
# (e.g. AlmaLinux 9 / RHEL9 ship glibc 2.34) even though this runner's own
# glibc is newer. Without this, a plain `cargo build` links against whatever
# glibc is installed on the CI host, which can be too new for target servers.
# Must be sourced (not executed) so its PATH/CARGO_ZIGBUILD_ZIG_PATH exports
# reach the `cargo zigbuild` call below.
. build/ci-ensure-zig.sh

case "$ARCH" in
  amd64) RUST_TARGET=x86_64-unknown-linux-gnu.2.28 ;;
  arm64) RUST_TARGET=aarch64-unknown-linux-gnu.2.28 ;;
esac
RUST_TARGET_DIR="${RUST_TARGET%%.*}"
rustup target add "${RUST_TARGET_DIR}" 2>/dev/null || true

NPROC="$(nproc 2>/dev/null || echo 16)"
# Leave a little headroom for the linker + OS on 32GB-class machines.
if [ "$NPROC" -ge 16 ]; then
  DEFAULT_JOBS=$((NPROC - 2))
else
  DEFAULT_JOBS="$NPROC"
fi
JOBS="${CARGO_BUILD_JOBS:-$DEFAULT_JOBS}"
export CARGO_BUILD_JOBS="$JOBS"
export CMAKE_BUILD_PARALLEL_LEVEL="${CMAKE_BUILD_PARALLEL_LEVEL:-$JOBS}"
export pertisk_proxy_VERSION="${VERSION#v}"
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"
# CI: no incremental cache across clean checkouts; slightly less disk/RAM churn.
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

echo "ci-release-native-bins: arch=${ARCH} jobs=${JOBS} target=${TARGET} rust_target=${RUST_TARGET} version=${pertisk_proxy_VERSION}"

copy_bin() {
  local bin="$1"
  cp "target/${RUST_TARGET_DIR}/release/${bin}" "./${bin}-linux-${ARCH}"
  chmod +x "./${bin}-linux-${ARCH}"
  file "./${bin}-linux-${ARCH}" || true
}

case "$TARGET" in
  proxy)
    cargo zigbuild --release --locked --target "${RUST_TARGET}" --bin pertisk-proxy
    copy_bin pertisk-proxy
    ;;
  ingress)
    cargo zigbuild --release --locked --target "${RUST_TARGET}" --bin pertisk-proxy-ingress --features ingress
    copy_bin pertisk-proxy-ingress
    ;;
  all)
    # Single graph: compile shared deps once, emit both bins.
    cargo zigbuild --release --locked --target "${RUST_TARGET}" \
      --bin pertisk-proxy \
      --bin pertisk-proxy-ingress \
      --features ingress
    copy_bin pertisk-proxy
    copy_bin pertisk-proxy-ingress
    ;;
  *)
    echo "TARGET must be proxy|ingress|all" >&2
    exit 1
    ;;
esac
