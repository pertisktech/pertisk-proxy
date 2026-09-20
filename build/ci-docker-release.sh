#!/usr/bin/env bash
# Build docker/Dockerfile.release and export binaries to a local directory.
# Usage: ./build/ci-docker-release.sh <linux/amd64|linux/arm64> <out-dir> [docker build-args...]
#
# Uses BuildKit -o type=local (no docker create on scratch — that fails with
# "no command specified"). Always --network=host for flaky mirror environments.
#
# Private registry pulls (RUST_IMAGE) require docker login / DOCKER_CONFIG auths.
# Cross builds use a docker-container buildx driver that reads DOCKER_CONFIG.
set -euo pipefail

PLATFORM="${1:?usage: $0 <linux/amd64|linux/arm64> <out-dir> [extra docker build args...]}"
OUT_DIR="${2:?}"
shift 2

case "$(uname -m)" in
  x86_64|amd64) NATIVE_ARCH=amd64; NATIVE_PLATFORM=linux/amd64 ;;
  aarch64|arm64) NATIVE_ARCH=arm64; NATIVE_PLATFORM=linux/arm64 ;;
  *) echo "unsupported runner arch: $(uname -m)" >&2; exit 1 ;;
esac

TARGET_ARCH="${PLATFORM#linux/}"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

RUST_IMAGE="${RUST_IMAGE:-registry.tools.thaidevops.co/pertisk-proxy/rust:1-bookworm}"

COMMON=(
  --file docker/Dockerfile.release
  --target artifacts
  --build-arg BUILDPLATFORM="$NATIVE_PLATFORM"
  --build-arg BUILDARCH="$NATIVE_ARCH"
  --build-arg TARGETPLATFORM="$PLATFORM"
  --build-arg TARGETARCH="$TARGET_ARCH"
  --build-arg "RUST_IMAGE=${RUST_IMAGE}"
  -o "type=local,dest=${OUT_DIR}"
  .
)

if [ "$TARGET_ARCH" = "$NATIVE_ARCH" ]; then
  echo "ci-docker-release: native build ($PLATFORM) → ${OUT_DIR}"
  # Pre-pull so auth failures are obvious (and populates local cache).
  docker pull --platform "$NATIVE_PLATFORM" "$RUST_IMAGE"
  DOCKER_BUILDKIT=1 docker build --network=host "${COMMON[@]}" "$@"
else
  echo "ci-docker-release: cross build ($PLATFORM on $NATIVE_PLATFORM) → ${OUT_DIR}"
  BUILDER=pertisk-release-cross
  # Recreate builder so it picks up current DOCKER_CONFIG registry auths.
  docker buildx rm -f "$BUILDER" >/dev/null 2>&1 || true
  docker buildx create --name "$BUILDER" --driver docker-container \
    --driver-opt network=host --use --bootstrap
  # Ensure the base image is reachable with credentials before BuildKit starts.
  docker pull --platform "$NATIVE_PLATFORM" "$RUST_IMAGE"
  docker buildx build --builder "$BUILDER" --platform "$PLATFORM" --network=host \
    "${COMMON[@]}" "$@"
fi

echo "ci-docker-release: exported:"
ls -la "$OUT_DIR"
