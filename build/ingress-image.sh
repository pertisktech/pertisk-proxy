#!/usr/bin/env bash
# Build and optionally push pertisk-proxy-ingress Docker image (single- or multi-arch).
#
# Usage:
#   ./build/ingress-image.sh [VERSION]
#   PUSH=1 ./build/ingress-image.sh
#   PLATFORMS=linux/amd64,linux/arm64 PUSH=1 ./build/ingress-image.sh
#
# Image: ${HARBOR_INGRESS_IMAGE}:${VERSION}

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

VERSION="${VERSION:-${1:-0.1.0}}"
IMAGE="${HARBOR_INGRESS_IMAGE:-harbor.tools.thaidevops.co/pertisksoft/pertisk-proxy/ingress}"
DOCKERFILE="${INGRESS_DOCKERFILE:-docker/Dockerfile.ingress}"
PLATFORMS="${PLATFORMS:-}"
PUSH="${PUSH:-${BUILD_PUSH:-}}"
BUILDER_NAME="${BUILDER_NAME:-pertisk-proxy-multiarch}"
CACHE_DIR="${CACHE_DIR:-.buildx-cache/ingress}"
CACHE_DIR_NEW="${CACHE_DIR_NEW:-.buildx-cache/ingress-new}"
CACHE_IMAGE="${CACHE_IMAGE:-${IMAGE}:buildcache}"
CACHE_BACKEND_RAW="${CACHE_BACKEND:-auto}"
CACHE_BACKEND="$(printf '%s' "$CACHE_BACKEND_RAW" | tr '[:upper:]' '[:lower:]' | xargs)"
SKIP_ADMIN_BUILD="${SKIP_ADMIN_BUILD:-0}"
PROVENANCE="${PROVENANCE:-false}"
SBOM="${SBOM:-false}"

if [ -z "$CACHE_BACKEND" ]; then
  CACHE_BACKEND="auto"
fi

if [ "$SKIP_ADMIN_BUILD" = "1" ]; then
  echo "Skipping admin build (SKIP_ADMIN_BUILD=1)."
else
  needs_admin_build=0
  if [ ! -d "admin/dist" ]; then
    needs_admin_build=1
  elif [ -n "$(find admin/src admin/public admin/index.html admin/package.json admin/pnpm-lock.yaml -type f -newer admin/dist 2>/dev/null | head -n 1)" ]; then
    needs_admin_build=1
  fi

  if [ "$needs_admin_build" -eq 1 ]; then
    echo "Building admin UI..."
    if [ ! -d "admin/node_modules" ]; then
      (cd admin && pnpm install)
    fi
    (cd admin && pnpm run build)
  else
    echo "admin/dist is up to date; skipping admin build."
  fi
fi

echo "Building ingress image: ${IMAGE}:${VERSION}"
export DOCKER_BUILDKIT=1

if ! docker buildx inspect "$BUILDER_NAME" >/dev/null 2>&1; then
  echo "Creating buildx builder '$BUILDER_NAME'..."
  docker buildx create --name "$BUILDER_NAME" --driver docker-container --bootstrap
fi

if [ "$CACHE_BACKEND" = "auto" ]; then
  if [ -n "$PLATFORMS" ]; then
    CACHE_BACKEND="registry"
  else
    CACHE_BACKEND="both"
  fi
fi

echo "Build config: platforms='${PLATFORMS:-single-arch}', dockerfile=${DOCKERFILE}, cache_backend=${CACHE_BACKEND}, push=${PUSH:-0}"

use_local_cache=0
cache_args=()
case "$CACHE_BACKEND" in
  registry)
    cache_args+=(--cache-from "type=registry,ref=${CACHE_IMAGE}")
    cache_args+=(--cache-from "type=registry,ref=${IMAGE}:latest")
    cache_args+=(--cache-to "type=registry,ref=${CACHE_IMAGE},mode=max,ignore-error=true")
    ;;
  local)
    use_local_cache=1
    ;;
  both)
    use_local_cache=1
    cache_args+=(--cache-from "type=registry,ref=${CACHE_IMAGE}")
    cache_args+=(--cache-from "type=registry,ref=${IMAGE}:latest")
    cache_args+=(--cache-to "type=registry,ref=${CACHE_IMAGE},mode=max,ignore-error=true")
    ;;
  none)
    ;;
  *)
    echo "Error: CACHE_BACKEND must be one of auto|registry|local|both|none" >&2
    exit 1
    ;;
esac

if [ "$use_local_cache" -eq 1 ]; then
  mkdir -p "$CACHE_DIR"
  rm -rf "$CACHE_DIR_NEW"
  cache_args+=(--cache-from "type=local,src=${CACHE_DIR}")
  cache_args+=(--cache-to "type=local,dest=${CACHE_DIR_NEW},mode=max,ignore-error=true")
fi

if [ -n "$PLATFORMS" ]; then
  if [ -z "$PUSH" ]; then
    echo "Error: multi-arch build requires PUSH=1 (manifest cannot be loaded locally)." >&2
    exit 1
  fi
  docker buildx build --builder "$BUILDER_NAME" --platform "$PLATFORMS" -f "$DOCKERFILE" \
    "${cache_args[@]}" \
    --provenance="$PROVENANCE" \
    --sbom="$SBOM" \
    -t "${IMAGE}:${VERSION}" \
    -t "${IMAGE}:latest" \
    --push \
    .
else
  docker buildx build --builder "$BUILDER_NAME" --load -f "$DOCKERFILE" \
    "${cache_args[@]}" \
    -t "${IMAGE}:${VERSION}" \
    -t "${IMAGE}:latest" \
    .
  if [ -n "$PUSH" ]; then
    docker push "${IMAGE}:${VERSION}"
    docker push "${IMAGE}:latest"
  fi
fi

if [ "$use_local_cache" -eq 1 ] && [ -d "$CACHE_DIR_NEW" ]; then
  rm -rf "$CACHE_DIR"
  mv "$CACHE_DIR_NEW" "$CACHE_DIR"
fi

echo "Done. Image: ${IMAGE}:${VERSION}"
if [ -n "$PLATFORMS" ]; then
  echo "Pushed multi-arch manifest ($(PLATFORMS)): ${IMAGE}:${VERSION}"
elif [ -n "$PUSH" ]; then
  echo "Pushed: ${IMAGE}:${VERSION}"
else
  echo "Built locally. Push: docker push ${IMAGE}:${VERSION}"
fi
