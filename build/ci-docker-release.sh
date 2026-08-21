#!/usr/bin/env bash
# Build docker/Dockerfile.release and export binaries to a local directory.
# Usage: ./build/ci-docker-release.sh <linux/amd64|linux/arm64> <out-dir> [docker build-args...]
#
# Uses BuildKit -o type=local (no docker create on scratch — that fails with
# "no command specified"). Always --network=host for flaky mirror environments.
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

COMMON=(
  --file docker/Dockerfile.release
  --target artifacts
  --build-arg BUILDPLATFORM="$NATIVE_PLATFORM"
  --build-arg BUILDARCH="$NATIVE_ARCH"
  --build-arg TARGETPLATFORM="$PLATFORM"
  --build-arg TARGETARCH="$TARGET_ARCH"
  --build-arg "RUST_IMAGE=${RUST_IMAGE:-public.ecr.aws/docker/library/rust:1-bookworm}"
  -o "type=local,dest=${OUT_DIR}"
  .
)

if [ "$TARGET_ARCH" = "$NATIVE_ARCH" ]; then
  echo "ci-docker-release: native build ($PLATFORM) → ${OUT_DIR}"
  DOCKER_BUILDKIT=1 docker build --network=host "${COMMON[@]}" "$@"
else
  echo "ci-docker-release: cross build ($PLATFORM on $NATIVE_PLATFORM) → ${OUT_DIR}"
  BUILDER=pertisk-release-cross
  if ! docker buildx inspect "$BUILDER" >/dev/null 2>&1; then
    docker buildx create --name "$BUILDER" --driver docker-container \
      --driver-opt network=host --bootstrap
  fi
  docker buildx build --builder "$BUILDER" --platform "$PLATFORM" --network=host \
    "${COMMON[@]}" "$@"
fi

echo "ci-docker-release: exported:"
ls -la "$OUT_DIR"
