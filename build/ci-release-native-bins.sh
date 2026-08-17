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

echo "ci-release-native-bins: arch=${ARCH} jobs=${JOBS} target=${TARGET} version=${pertisk_proxy_VERSION}"

copy_bin() {
  local bin="$1"
  cp "target/release/${bin}" "./${bin}-linux-${ARCH}"
  chmod +x "./${bin}-linux-${ARCH}"
  file "./${bin}-linux-${ARCH}" || true
}

case "$TARGET" in
  proxy)
    cargo build --release --locked --bin pertisk-proxy
    copy_bin pertisk-proxy
    ;;
  ingress)
    cargo build --release --locked --bin pertisk-proxy-ingress --features ingress
    copy_bin pertisk-proxy-ingress
    ;;
  all)
    # Single graph: compile shared deps once, emit both bins.
    cargo build --release --locked \
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
