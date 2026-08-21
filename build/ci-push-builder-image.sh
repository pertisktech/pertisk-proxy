#!/usr/bin/env bash
# Build/push Harbor musl builder (rust + apk deps + cargo-zigbuild).
# Run from a machine that can reach Alpine mirrors + Harbor.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

REGISTRY="${CONTAINER_REGISTRY:-harbor.tools.pertisk.com}"
BUILDER="${HARBOR_BUILDER_IMAGE:-${REGISTRY}/pertisk-proxy/builder}"
TAG="${BUILDER_TAG:-alpine-rust}"
RUST_SRC="${RUST_SRC:-${REGISTRY}/pertisk-proxy/rust:1-alpine3.21}"
# Single-arch first if multi-arch apk is slow; default both.
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"

echo "Pushing ${BUILDER}:${TAG} (platforms=${PLATFORMS}, rust=${RUST_SRC})"
docker buildx build \
  --platform "${PLATFORMS}" \
  --provenance=false \
  -f docker/Dockerfile.builder \
  --build-arg "RUST_SRC=${RUST_SRC}" \
  -t "${BUILDER}:${TAG}" \
  -t "${BUILDER}:latest" \
  --push \
  .

echo "Done. CI RUST_IMAGE=${BUILDER}:${TAG}"
docker buildx imagetools inspect "${BUILDER}:${TAG}" | head -25
