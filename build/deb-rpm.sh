#!/usr/bin/env bash
# Run inside docker/Dockerfile.package. Builds .deb and .rpm for each binary in PACKAGE_BINARIES.
set -euo pipefail
cd /work

IFS=' ' read -r -a BINS <<< "${PACKAGE_BINARIES}"

for BINARY_NAME in "${BINS[@]}"; do
  fpm -s dir -t deb --force \
    -n "$BINARY_NAME" \
    -v "$VERSION" \
    -a "$deb_arch" \
    --description "pertisk-proxy ($BINARY_NAME)" \
    --url "https://github.com/pertisktech/pertisk-proxy" \
    --maintainer "Pertisk Team" \
    --license "Apache-2.0" \
    --vendor "Pertisk" \
    --category "net" \
    --depends libcap2-bin \
    --before-install /work/preinstall.sh \
    --after-install /work/postinstall.sh \
    --before-remove /work/preremove.sh \
    --config-files "/etc/pertisk-proxy/${BINARY_NAME}.conf" \
    --directories /var/lib/pertisk-proxy \
    --directories /var/log/pertisk-proxy \
    --deb-systemd-enable \
    --deb-no-default-config-files \
    -p /work/release \
    -C "/work/pkg-${BINARY_NAME}" .

  fpm -s dir -t rpm --force \
    -n "$BINARY_NAME" \
    -v "$VERSION" \
    -a "$rpm_arch" \
    --description "pertisk-proxy ($BINARY_NAME)" \
    --url "https://github.com/pertisktech/pertisk-proxy" \
    --maintainer "Pertisk Team" \
    --license "Apache-2.0" \
    --vendor "Pertisk" \
    --category "System Environment/Daemons" \
    --depends libcap \
    --depends shadow-utils \
    --before-install /work/preinstall.sh \
    --after-install /work/postinstall.sh \
    --before-remove /work/preremove.sh \
    --config-files "/etc/pertisk-proxy/${BINARY_NAME}.conf" \
    --directories /var/lib/pertisk-proxy \
    --directories /var/log/pertisk-proxy \
    --rpm-os linux \
    -p /work/release \
    -C "/work/pkg-${BINARY_NAME}" .
done
