#!/usr/bin/env bash
set -euo pipefail
cd /work
IFS=' ' read -r -a BINS <<< "${PACKAGE_BINARIES}"
for BINARY_NAME in "${BINS[@]}"; do
  if [ "$BINARY_NAME" = "pertisk-tunnel-server" ]; then
    CONF="/etc/pertisk-tunnel/server.toml"
  else
    CONF="/etc/pertisk-tunnel/client.toml"
  fi
  fpm -s dir -t deb --force \
    -n "$BINARY_NAME" -v "$VERSION" -a "$deb_arch" \
    --description "pertisk reverse tunnel ($BINARY_NAME)" \
    --url "https://github.com/pertisktech/pertisk-proxy" \
    --maintainer "Pertisk Team" --license "Apache-2.0" --vendor "Pertisk" \
    --category net \
    --before-install /work/preinstall-tunnel.sh \
    --after-install /work/postinstall-tunnel.sh \
    --before-remove /work/preremove-tunnel.sh \
    --config-files "$CONF" \
    --directories /etc/pertisk-tunnel \
    --directories /var/lib/pertisk-tunnel \
    --deb-systemd-enable --deb-no-default-config-files \
    -p /work/release -C "/work/pkg-${BINARY_NAME}" .

  fpm -s dir -t rpm --force \
    -n "$BINARY_NAME" -v "$VERSION" -a "$rpm_arch" \
    --description "pertisk reverse tunnel ($BINARY_NAME)" \
    --url "https://github.com/pertisktech/pertisk-proxy" \
    --maintainer "Pertisk Team" --license "Apache-2.0" --vendor "Pertisk" \
    --category "System Environment/Daemons" \
    --depends shadow-utils \
    --before-install /work/preinstall-tunnel.sh \
    --after-install /work/postinstall-tunnel.sh \
    --before-remove /work/preremove-tunnel.sh \
    --config-files "$CONF" \
    --directories /etc/pertisk-tunnel \
    --directories /var/lib/pertisk-tunnel \
    --rpm-os linux \
    -p /work/release -C "/work/pkg-${BINARY_NAME}" .
done
