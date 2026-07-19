#!/usr/bin/env bash
# Build and optionally push pertisk-proxy-ingress Docker image (single- or multi-arch).
#
# Usage:
#   ./build/ingress-image.sh [VERSION]
#   PUSH=1 ./build/ingress-image.sh
#   PLATFORMS=linux/amd64,linux/arm64 PUSH=1 ./build/ingress-image.sh
#
# PUSH=1 defaults to linux/amd64,linux/arm64 so registry tags stay multi-arch manifests.
# Kubernetes/containerd then auto-selects the node architecture on pull (no nodeSelector needed).
#
# Image: ${HARBOR_INGRESS_IMAGE}:${VERSION}

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

VERSION="${VERSION:-${1:-0.1.0}}"
IMAGE="${HARBOR_INGRESS_IMAGE:-harbor.tools.thaidevops.co/pertisksoft/pertisk-proxy/ingress}"
DOCKERFILE="${INGRESS_DOCKERFILE:-docker/Dockerfile.ingress}"
DEFAULT_PLATFORMS="${DEFAULT_PLATFORMS:-linux/amd64,linux/arm64}"
PLATFORMS="${PLATFORMS:-}"
PUSH="${PUSH:-${BUILD_PUSH:-}}"

# Any registry push must publish a manifest list so clusters pull the matching arch automatically.
if [ -n "$PUSH" ] && [ -z "$PLATFORMS" ]; then
  PLATFORMS="$DEFAULT_PLATFORMS"
fi
BUILDER_NAME="${BUILDER_NAME:-pertisk-proxy-multiarch}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
PARALLEL_PLATFORMS="${PARALLEL_PLATFORMS:-1}"
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

echo "Build config: platforms='${PLATFORMS:-single-arch}', dockerfile=${DOCKERFILE}, cache_backend=${CACHE_BACKEND}, push=${PUSH:-0}, cargo_jobs=${CARGO_BUILD_JOBS}, parallel=${PARALLEL_PLATFORMS}"

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

verify_ingress_manifest() {
  local tag="$1"
  local pull_policy="$2"
  shift 2
  local platforms=("$@")
  local platform arch file_out cid pull_args=()
  if [ "$pull_policy" = "always" ] || [ "$pull_policy" = "missing" ]; then
    pull_args=(--pull "$pull_policy")
  fi
  for platform in "${platforms[@]}"; do
    platform="${platform// /}"
    arch="${platform#linux/}"
    cid="$(docker create "${pull_args[@]}" --platform "$platform" "${IMAGE}:${tag}")"
    docker cp "${cid}:/usr/local/bin/pertisk-proxy-ingress" "/tmp/pertisk-ingress-${arch}" >/dev/null
    docker rm "${cid}" >/dev/null
    file_out="$(file "/tmp/pertisk-ingress-${arch}")"
    rm -f "/tmp/pertisk-ingress-${arch}"
    case "$arch" in
      amd64) echo "$file_out" | grep -Eq 'x86-64|Intel 80386' || { echo "Error: ${IMAGE}:${tag} ${platform} binary check failed: $file_out" >&2; return 1; } ;;
      arm64) echo "$file_out" | grep -Eq 'aarch64|ARM' || { echo "Error: ${IMAGE}:${tag} ${platform} binary check failed: $file_out" >&2; return 1; } ;;
      *) echo "Error: unsupported platform for verify: ${platform}" >&2; return 1 ;;
    esac
    echo "Verified ${platform} binary (${tag}): ${file_out}"
  done
}

build_ingress_platform() {
  local platform="$1"
  platform="${platform// /}"
  local arch="${platform#linux/}"
  local platform_tag="${IMAGE}:${VERSION}-${arch}"
  local platform_cache="${CACHE_IMAGE}-${arch}"
  local -a per_platform_cache_args=()

  case "$CACHE_BACKEND" in
    registry)
      per_platform_cache_args+=(--cache-from "type=registry,ref=${platform_cache}")
      per_platform_cache_args+=(--cache-to "type=registry,ref=${platform_cache},mode=max,ignore-error=true")
      ;;
    local)
      mkdir -p "${CACHE_DIR}/${arch}"
      rm -rf "${CACHE_DIR_NEW}/${arch}"
      per_platform_cache_args+=(--cache-from "type=local,src=${CACHE_DIR}/${arch}")
      per_platform_cache_args+=(--cache-to "type=local,dest=${CACHE_DIR_NEW}/${arch},mode=max,ignore-error=true")
      ;;
    both)
      mkdir -p "${CACHE_DIR}/${arch}"
      rm -rf "${CACHE_DIR_NEW}/${arch}"
      per_platform_cache_args+=(--cache-from "type=registry,ref=${platform_cache}")
      per_platform_cache_args+=(--cache-to "type=registry,ref=${platform_cache},mode=max,ignore-error=true")
      per_platform_cache_args+=(--cache-from "type=local,src=${CACHE_DIR}/${arch}")
      per_platform_cache_args+=(--cache-to "type=local,dest=${CACHE_DIR_NEW}/${arch},mode=max,ignore-error=true")
      ;;
    none)
      ;;
  esac

  echo "Building ${platform} -> ${platform_tag} (native cross-compile, jobs=${CARGO_BUILD_JOBS})"
  if [ "${#per_platform_cache_args[@]}" -gt 0 ]; then
    docker buildx build --builder "$BUILDER_NAME" --platform "$platform" -f "$DOCKERFILE" \
      "${per_platform_cache_args[@]}" \
      --build-arg "TARGETPLATFORM=${platform}" \
      --build-arg "TARGETARCH=${arch}" \
      --build-arg "VERSION=${VERSION}" \
      --build-arg "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}" \
      --provenance="$PROVENANCE" \
      --sbom="$SBOM" \
      -t "$platform_tag" \
      --push \
      .
  else
    docker buildx build --builder "$BUILDER_NAME" --platform "$platform" -f "$DOCKERFILE" \
      --build-arg "TARGETPLATFORM=${platform}" \
      --build-arg "TARGETARCH=${arch}" \
      --build-arg "VERSION=${VERSION}" \
      --build-arg "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}" \
      --provenance="$PROVENANCE" \
      --sbom="$SBOM" \
      -t "$platform_tag" \
      --push \
      .
  fi
}

if [ -n "$PLATFORMS" ]; then
  if [ -z "$PUSH" ]; then
    echo "Error: multi-arch build requires PUSH=1 (manifest cannot be loaded locally)." >&2
    exit 1
  fi
  IFS=',' read -r -a PLATFORM_LIST <<< "$PLATFORMS"
  platform_tags=()
  build_pids=()
  for platform in "${PLATFORM_LIST[@]}"; do
    platform="${platform// /}"
    arch="${platform#linux/}"
    platform_tags+=("${IMAGE}:${VERSION}-${arch}")
    if [ "$PARALLEL_PLATFORMS" = "1" ] && [ "${#PLATFORM_LIST[@]}" -gt 1 ]; then
      build_ingress_platform "$platform" &
      build_pids+=("$!")
    else
      build_ingress_platform "$platform"
    fi
  done
  if [ "${#build_pids[@]}" -gt 0 ]; then
    for pid in "${build_pids[@]}"; do
      wait "$pid"
    done
  fi
  if [ "$use_local_cache" -eq 1 ] && [ -d "$CACHE_DIR_NEW" ]; then
    rm -rf "$CACHE_DIR"
    mv "$CACHE_DIR_NEW" "$CACHE_DIR"
  fi

  # Verify pushed per-arch images before publishing the manifest list.
  for platform in "${PLATFORM_LIST[@]}"; do
    platform="${platform// /}"
    arch="${platform#linux/}"
    verify_ingress_manifest "${VERSION}-${arch}" "always" "$platform"
  done

  if [ "${#platform_tags[@]}" -eq 1 ]; then
    docker buildx imagetools create \
      -t "${IMAGE}:${VERSION}" \
      -t "${IMAGE}:latest" \
      "${platform_tags[0]}"
  else
    docker buildx imagetools create \
      -t "${IMAGE}:${VERSION}" \
      -t "${IMAGE}:latest" \
      "${platform_tags[@]}"
  fi

  if docker buildx imagetools inspect "${IMAGE}:${VERSION}" >/tmp/pertisk-ingress-manifest-inspect.txt 2>&1; then
    echo "Published manifest ${IMAGE}:${VERSION}:"
    grep -E '^Name:|^MediaType:|^Digest:|^Platform:' /tmp/pertisk-ingress-manifest-inspect.txt \
      || cat /tmp/pertisk-ingress-manifest-inspect.txt
    rm -f /tmp/pertisk-ingress-manifest-inspect.txt
  fi

  # Drop stale local manifest copy; re-pull from registry before checking the merged tag.
  docker rmi "${IMAGE}:${VERSION}" "${IMAGE}:latest" 2>/dev/null || true
  verify_ingress_manifest "${VERSION}" "always" "${PLATFORM_LIST[@]}"
else
  case "$(uname -m)" in
    x86_64) NATIVE_ARCH=amd64 ;;
    aarch64|arm64) NATIVE_ARCH=arm64 ;;
    *) echo "Error: unsupported native arch: $(uname -m)" >&2; exit 1 ;;
  esac
  docker buildx build --builder "$BUILDER_NAME" --load -f "$DOCKERFILE" \
    "${cache_args[@]}" \
    --build-arg "TARGETPLATFORM=linux/${NATIVE_ARCH}" \
    --build-arg "TARGETARCH=${NATIVE_ARCH}" \
    --build-arg "VERSION=${VERSION}" \
    --build-arg "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}" \
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
if [ -n "$PLATFORMS" ] && [ -n "$PUSH" ]; then
  echo "Pushed multi-arch manifest (${PLATFORMS}): ${IMAGE}:${VERSION}"
  if docker buildx imagetools inspect "${IMAGE}:${VERSION}" >/tmp/pertisk-ingress-manifest.txt 2>&1; then
    echo "Registry manifest (kubelet pulls matching platform automatically):"
    grep -E '^Name:|^MediaType:|^Platform:' /tmp/pertisk-ingress-manifest.txt || cat /tmp/pertisk-ingress-manifest.txt
    rm -f /tmp/pertisk-ingress-manifest.txt
  fi
elif [ -n "$PUSH" ]; then
  echo "Pushed: ${IMAGE}:${VERSION}"
else
  echo "Built locally (native arch). Push multi-arch: PUSH=1 ./build/ingress-image.sh ${VERSION}"
fi
