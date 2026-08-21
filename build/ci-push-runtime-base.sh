#!/usr/bin/env bash
# Build and push Harbor-hosted runtime base used by release docker-build.
# Run from a machine that CAN reach Public ECR / Docker Hub (not the locked-down CI runner).
#
#   ./build/ci-push-runtime-base.sh
#   DEBIAN_SRC=debian:bookworm-slim ./build/ci-push-runtime-base.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

REGISTRY="${CONTAINER_REGISTRY:-harbor.tools.pertisk.com}"
RUNTIME_IMAGE="${HARBOR_RUNTIME_IMAGE:-${REGISTRY}/pertisk-proxy/runtime}"
TAG="${RUNTIME_TAG:-bookworm}"
DEBIAN_SRC="${DEBIAN_SRC:-public.ecr.aws/docker/library/debian:bookworm-slim}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"

echo "Pushing ${RUNTIME_IMAGE}:${TAG} (platforms=${PLATFORMS}, src=${DEBIAN_SRC})"
docker buildx build \
  --platform "${PLATFORMS}" \
  --provenance=false \
  -f docker/Dockerfile.runtime \
  --build-arg "DEBIAN_SRC=${DEBIAN_SRC}" \
  -t "${RUNTIME_IMAGE}:${TAG}" \
  -t "${RUNTIME_IMAGE}:latest" \
  --push \
  .

echo "Done. CI should use DEBIAN_IMAGE=${RUNTIME_IMAGE}:${TAG}"
docker buildx imagetools inspect "${RUNTIME_IMAGE}:${TAG}" | head -30
