#!/usr/bin/env bash
# Mirror public builder/runtime bases into Harbor so locked-down CI runners never
# need docker.io / public.ecr.aws DNS.
#
# Run from a machine with internet + Harbor login:
#   ./build/ci-mirror-base-images.sh
#   ./build/ci-push-runtime-base.sh   # Debian+openssl runtime (apt baked in)
set -euo pipefail

REGISTRY="${CONTAINER_REGISTRY:-harbor.tools.pertisk.com}"
PROJECT="${HARBOR_PROJECT:-pertisk-proxy}"
SRC_PREFIX="${BASE_SRC_PREFIX:-public.ecr.aws/docker/library}"

mirror() {
  local src="$1"
  local dst="$2"
  echo "Mirror ${src} -> ${dst}"
  docker buildx imagetools create --tag "$dst" "$src"
  docker buildx imagetools inspect "$dst" | head -15
  echo
}

mirror "${SRC_PREFIX}/rust:1-alpine3.21" "${REGISTRY}/${PROJECT}/rust:1-alpine3.21"
mirror "${SRC_PREFIX}/alpine:3.21" "${REGISTRY}/${PROJECT}/alpine:3.21"
mirror "${SRC_PREFIX}/rust:1-bookworm" "${REGISTRY}/${PROJECT}/rust:1-bookworm"
mirror "${SRC_PREFIX}/debian:bookworm-slim" "${REGISTRY}/${PROJECT}/debian:bookworm-slim"

echo "Done. Point CI at:"
echo "  RUST_IMAGE=${REGISTRY}/${PROJECT}/rust:1-alpine3.21"
echo "  ALPINE_IMAGE=${REGISTRY}/${PROJECT}/alpine:3.21"
echo "  RUST_BOOKWORM_IMAGE=${REGISTRY}/${PROJECT}/rust:1-bookworm"
echo "  + ./build/ci-push-runtime-base.sh for runtime:bookworm"
