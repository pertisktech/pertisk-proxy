#!/usr/bin/env bash
# Build DEB/RPM with fpm. Uses Docker when fpm is not on PATH (no sudo/ruby required).
set -euo pipefail

: "${BINARY_NAME:?BINARY_NAME required}"
: "${VERSION:?VERSION required}"
: "${deb_arch:?deb_arch required}"
: "${rpm_arch:?rpm_arch required}"
: "${PKG_DIR:=pkg}"

PACKAGE_IMAGE="${PACKAGE_IMAGE:-pertisk-proxy-package}"

run_fpm() {
  if command -v fpm >/dev/null 2>&1; then
    fpm "$@"
    return
  fi

  if ! command -v docker >/dev/null 2>&1; then
    echo "::error::fpm not found and docker is unavailable for packaging container" >&2
    exit 1
  fi

  docker build -q -f docker/Dockerfile.package -t "$PACKAGE_IMAGE" .
  docker run --rm -v "$(pwd):/work" -w /work "$PACKAGE_IMAGE" fpm "$@"
}

common_args=(
  -s dir
  -n "$BINARY_NAME"
  -v "v${VERSION}"
  --description "Reverse proxy and Kubernetes Ingress controller with HTTP/3"
  --url "https://github.com/pertisktech/pertisk-proxy"
  --maintainer "Pertisk Team"
  --license "Apache-2.0"
  --vendor "Pertisk"
  --before-install preinstall.sh
  --after-install postinstall.sh
  --before-remove preremove.sh
  --config-files "/etc/pertisk-proxy/pertisk-proxy.conf"
  --directories /var/lib/pertisk-proxy
  --directories /var/log/pertisk-proxy
  -C "$PKG_DIR"
  .
)

run_fpm -t deb -a "$deb_arch" \
  --category net \
  --depends libcap2-bin \
  --deb-systemd-enable \
  --deb-no-default-config-files \
  "${common_args[@]}"

run_fpm -t rpm -a "$rpm_arch" \
  --category "System Environment/Daemons" \
  --depends libcap \
  --depends shadow-utils \
  --rpm-os linux \
  "${common_args[@]}"
