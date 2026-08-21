#!/usr/bin/env bash
# Assemble multi-arch Harbor images from CI package binaries (no rust/node base pulls).
#
# Expects in cwd:
#   pertisk-proxy-linux-{amd64,arm64}
#   pertisk-proxy-ingress-linux-{amd64,arm64}
#   admin/dist/
#
# Env:
#   VERSION (required)
#   HARBOR_PROXY_IMAGE / HARBOR_INGRESS_IMAGE
#   DEBIAN_IMAGE — Harbor runtime base (default …/pertisk-proxy/runtime:bookworm)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

: "${VERSION:?VERSION required}"

PROXY="${HARBOR_PROXY_IMAGE:-harbor.tools.pertisk.com/pertisk-proxy/proxy}"
INGRESS="${HARBOR_INGRESS_IMAGE:-harbor.tools.pertisk.com/pertisk-proxy/ingress}"
DEBIAN_IMAGE="${DEBIAN_IMAGE:-harbor.tools.pertisk.com/pertisk-proxy/runtime:bookworm}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
PROVENANCE="${PROVENANCE:-false}"

if [ ! -f admin/dist/index.html ]; then
  echo "Error: admin/dist/index.html missing" >&2
  exit 1
fi

# Confirm Harbor runtime base is pullable before building (fail fast with a clear hint).
if ! docker buildx imagetools inspect "${DEBIAN_IMAGE}" >/dev/null 2>&1; then
  echo "Error: cannot inspect runtime base ${DEBIAN_IMAGE}" >&2
  echo "Push it from a networked machine: ./build/ci-push-runtime-base.sh" >&2
  exit 1
fi

IFS=',' read -r -a PLATFORM_LIST <<< "$PLATFORMS"

build_one() {
  local kind="$1" # proxy|ingress
  local platform="$2"
  local arch="${platform#linux/}"
  local dockerfile binary image tag

  case "$kind" in
    proxy)
      dockerfile=docker/Dockerfile.binary
      binary="pertisk-proxy-linux-${arch}"
      image="$PROXY"
      ;;
    ingress)
      dockerfile=docker/Dockerfile.ingress.binary
      binary="pertisk-proxy-ingress-linux-${arch}"
      image="$INGRESS"
      ;;
    *) echo "bad kind: $kind" >&2; exit 1 ;;
  esac

  if [ ! -f "$binary" ]; then
    echo "Error: missing ${binary}" >&2
    exit 1
  fi

  tag="${image}:${VERSION}-${arch}"
  echo "Building ${tag} (${platform}) from ${binary}"
  docker buildx build \
    --platform "$platform" \
    --provenance="$PROVENANCE" \
    --network=host \
    -f "$dockerfile" \
    --build-arg "DEBIAN_IMAGE=${DEBIAN_IMAGE}" \
    --build-arg "BINARY_FILE=${binary}" \
    -t "$tag" \
    --push \
    .
}

proxy_tags=()
ingress_tags=()
for platform in "${PLATFORM_LIST[@]}"; do
  platform="${platform// /}"
  arch="${platform#linux/}"
  build_one proxy "$platform"
  build_one ingress "$platform"
  proxy_tags+=("${PROXY}:${VERSION}-${arch}")
  ingress_tags+=("${INGRESS}:${VERSION}-${arch}")
done

echo "Creating multi-arch manifests for ${VERSION}"
docker buildx imagetools create \
  -t "${PROXY}:${VERSION}" \
  -t "${PROXY}:v${VERSION}" \
  -t "${PROXY}:latest" \
  "${proxy_tags[@]}"

docker buildx imagetools create \
  -t "${INGRESS}:${VERSION}" \
  -t "${INGRESS}:v${VERSION}" \
  -t "${INGRESS}:latest" \
  "${ingress_tags[@]}"

echo "Published:"
echo "  ${PROXY}:${VERSION} (+ v${VERSION}, latest)"
echo "  ${INGRESS}:${VERSION} (+ v${VERSION}, latest)"
docker buildx imagetools inspect "${INGRESS}:${VERSION}" | head -25
