#!/usr/bin/env bash
# Build docker/Dockerfile.release and load a tagged image.
# Same-arch: host docker build (avoids buildkit container apt/GPG failures).
# Cross-arch: buildx with docker-container driver + network=host.
set -euo pipefail

PLATFORM="${1:?usage: $0 <linux/amd64|linux/arm64> <image-tag> [extra docker build args...]}"
IMAGE_TAG="${2:?}"
shift 2

case "$(uname -m)" in
  x86_64|amd64) NATIVE_ARCH=amd64; NATIVE_PLATFORM=linux/amd64 ;;
  aarch64|arm64) NATIVE_ARCH=arm64; NATIVE_PLATFORM=linux/arm64 ;;
  *) echo "unsupported runner arch: $(uname -m)" >&2; exit 1 ;;
esac

TARGET_ARCH="${PLATFORM#linux/}"
COMMON=(
  --file docker/Dockerfile.release
  --build-arg BUILDPLATFORM="$NATIVE_PLATFORM"
  --build-arg BUILDARCH="$NATIVE_ARCH"
  --build-arg TARGETPLATFORM="$PLATFORM"
  --build-arg TARGETARCH="$TARGET_ARCH"
  -t "$IMAGE_TAG"
  .
)

if [ "$TARGET_ARCH" = "$NATIVE_ARCH" ]; then
  echo "ci-docker-release: native docker build ($PLATFORM)"
  DOCKER_BUILDKIT=1 docker build "${COMMON[@]}" "$@"
else
  echo "ci-docker-release: buildx cross build ($PLATFORM on $NATIVE_PLATFORM)"
  BUILDER=pertisk-release-cross
  if ! docker buildx inspect "$BUILDER" >/dev/null 2>&1; then
    docker buildx create --name "$BUILDER" --driver docker-container \
      --driver-opt network=host --bootstrap
  fi
  docker buildx build --builder "$BUILDER" --platform "$PLATFORM" --load "${COMMON[@]}" "$@"
fi
