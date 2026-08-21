#!/usr/bin/env bash
# Push Harbor alpine runtime (openssl + ca-certs) for CI Dockerfile final stage.
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

REGISTRY="${CONTAINER_REGISTRY:-harbor.tools.pertisk.com}"
IMAGE="${HARBOR_ALPINE_RUNTIME:-${REGISTRY}/pertisk-proxy/runtime}"
TAG="${ALPINE_RUNTIME_TAG:-alpine}"
ALPINE_SRC="${ALPINE_SRC:-${REGISTRY}/pertisk-proxy/alpine:3.21}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"

echo "Pushing ${IMAGE}:${TAG} (src=${ALPINE_SRC})"
docker buildx build \
  --platform "${PLATFORMS}" \
  --provenance=false \
  -f docker/Dockerfile.runtime-alpine \
  --build-arg "ALPINE_SRC=${ALPINE_SRC}" \
  -t "${IMAGE}:${TAG}" \
  --push \
  .
docker buildx imagetools inspect "${IMAGE}:${TAG}" | head -20
