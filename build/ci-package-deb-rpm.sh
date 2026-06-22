#!/usr/bin/env bash
# Build DEB/RPM with fpm. In CI always uses Docker (host fpm often lacks rpmbuild).
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

run_fpm() {
  # CI runners may have fpm but not rpmbuild; RPM would fail or be skipped downstream.
  if [ "${CI:-}" = "true" ] || [ "${FORCE_DOCKER_FPM:-0}" = "1" ]; then
    :
  elif command -v fpm >/dev/null 2>&1 && command -v rpmbuild >/dev/null 2>&1; then
    fpm "$@"
    return
  fi

  if ! command -v docker >/dev/null 2>&1; then
    echo "::error::fpm/rpmbuild not available and docker is unavailable for packaging container" >&2
    exit 1
  fi

  docker build -q -f docker/Dockerfile.package -t "$PACKAGE_IMAGE" .
  docker run --rm -u "$(id -u):$(id -g)" -v "$(pwd):/work" -w /work "$PACKAGE_IMAGE" fpm "$@"
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
  --deb-systemd-enable \
  --deb-no-default-config-files \
  "${common_args[@]}"

echo "=== Building RPM (${rpm_arch}) ==="
run_fpm -t rpm -a "$rpm_arch" \
  --category "System Environment/Daemons" \
  --depends libcap \
  --depends shadow-utils \
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
