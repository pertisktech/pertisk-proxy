#!/usr/bin/env bash
# Build DEB/RPM with fpm. Prefers host fpm+rpmbuild; else Docker (AlmaLinux image).
set -euo pipefail

: "${BINARY_NAME:?BINARY_NAME required}"
: "${VERSION:?VERSION required}"
: "${deb_arch:?deb_arch required}"
: "${rpm_arch:?rpm_arch required}"
: "${PKG_DIR:=pkg}"
: "${OUTPUT_DIR:=release}"

PACKAGE_IMAGE="${PACKAGE_IMAGE:-pertisk-proxy-package}"
REPO_URL="${GITHUB_REPOSITORY:+https://github.com/${GITHUB_REPOSITORY}}"
REPO_URL="${REPO_URL:-https://github.com/pertisktech/pertisk-proxy}"

mkdir -p "$OUTPUT_DIR"

host_fpm_ok() {
  command -v fpm >/dev/null 2>&1 && command -v rpmbuild >/dev/null 2>&1
}

ensure_package_image() {
  echo "Building packaging image ${PACKAGE_IMAGE} (AlmaLinux + fpm)..."
  docker build --network=host --progress=plain \
    -f docker/Dockerfile.package -t "$PACKAGE_IMAGE" .
}

run_fpm() {
  if [ "${FORCE_DOCKER_FPM:-0}" != "1" ] && host_fpm_ok; then
    echo "fpm: using host ($(command -v fpm))"
    fpm "$@"
    return
  fi

  if ! command -v docker >/dev/null 2>&1; then
    echo "::error::fpm/rpmbuild not available and docker is unavailable for packaging container" >&2
    exit 1
  fi

  ensure_package_image
  echo "fpm: using docker image ${PACKAGE_IMAGE}"
  docker run --rm --network=host \
    -u "$(id -u):$(id -g)" \
    -v "$(pwd):/work" -w /work \
    "$PACKAGE_IMAGE" fpm "$@"
}

common_args=(
  -s dir
  -n "$BINARY_NAME"
  -v "${VERSION}"
  --iteration 1
  --description "Reverse proxy and Kubernetes Ingress controller with HTTP/3"
  --url "$REPO_URL"
  --maintainer "Pertisk Team"
  --license "Apache-2.0"
  --vendor "Pertisk"
  --before-install preinstall.sh
  --after-install postinstall.sh
  --before-remove preremove.sh
  --config-files "/etc/pertisk-proxy/pertisk-proxy.conf"
  --directories /var/lib/pertisk-proxy
  --directories /var/log/pertisk-proxy
  -p "$OUTPUT_DIR"
  -C "$PKG_DIR"
  .
)

echo "=== Building DEB (${deb_arch}) ==="
run_fpm -t deb -a "$deb_arch" \
  --category net \
  --depends libcap2-bin \
  --depends libssl3 \
  --deb-systemd-enable \
  --deb-no-default-config-files \
  "${common_args[@]}"

echo "=== Building RPM (${rpm_arch}) ==="
run_fpm -t rpm -a "$rpm_arch" \
  --category "System Environment/Daemons" \
  --depends libcap \
  --depends shadow-utils \
  --depends openssl-libs \
  --rpm-os linux \
  "${common_args[@]}"

echo "=== Package artifacts in ${OUTPUT_DIR}/ ==="
ls -la "${OUTPUT_DIR}/"

deb_count=$(find "$OUTPUT_DIR" -maxdepth 1 -name '*.deb' | wc -l | tr -d ' ')
rpm_count=$(find "$OUTPUT_DIR" -maxdepth 1 -name '*.rpm' | wc -l | tr -d ' ')
if [ "$deb_count" -lt 1 ] || [ "$rpm_count" -lt 1 ]; then
  echo "::error::Expected 1 DEB and 1 RPM in ${OUTPUT_DIR}/, got deb=${deb_count} rpm=${rpm_count}" >&2
  exit 1
fi
