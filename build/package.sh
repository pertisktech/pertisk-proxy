#!/usr/bin/env bash
# Build DEB, RPM, and tarball for pertisk-proxy (proxy mode) and/or ingress mode.
# Usage: ./build/package.sh <amd64|arm64> [VERSION] [proxy|ingress|all]
# Run from repo root.

set -euo pipefail

ARCH="${1:?Usage: $0 <amd64|arm64> [VERSION] [proxy|ingress|all]}"
VERSION="${2:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"
VERSION="${VERSION#v}"
TARGET="${3:-all}"
RELEASE_DIR="release"
mkdir -p "$RELEASE_DIR"

case "$ARCH" in
  amd64) deb_arch=amd64; rpm_arch=x86_64 ;;
  arm64) deb_arch=arm64; rpm_arch=aarch64 ;;
  *) echo "Error: ARCH must be amd64 or arm64" >&2; exit 1 ;;
esac

build_binary() {
  local name="$1"
  local features="${2:-}"
  local out="${name}-linux-${ARCH}"

  if [ -f "$out" ]; then
    echo "Using existing $out"
    return
  fi

  echo "Building $name for linux/$ARCH (version $VERSION)..."
  if [ -n "$features" ]; then
    cargo build --release --bin "$name" --features "$features"
  else
    cargo build --release --bin "$name"
  fi
  cp "target/release/$name" "./$out"
  chmod +x "./$out"
}

case "$TARGET" in
  proxy)
    build_binary pertisk-proxy
    PACKAGE_BINARIES=(pertisk-proxy)
    ;;
  ingress)
    build_binary pertisk-proxy-ingress ingress
    PACKAGE_BINARIES=(pertisk-proxy-ingress)
    ;;
  all|*)
    build_binary pertisk-proxy
    build_binary pertisk-proxy-ingress ingress
    PACKAGE_BINARIES=(pertisk-proxy pertisk-proxy-ingress)
    ;;
esac

for bin in "${PACKAGE_BINARIES[@]}"; do
  cp "${bin}-linux-${ARCH}" "$RELEASE_DIR/"
done

make_pkg_layout() {
  local bin="$1"
  local unit="$2"
  local conf="$3"
  local desc="$4"

  rm -rf "pkg-${bin}"
  mkdir -p "pkg-${bin}/usr/bin" \
    "pkg-${bin}/etc/pertisk-proxy" \
    "pkg-${bin}/var/lib/pertisk-proxy" \
    "pkg-${bin}/var/log/pertisk-proxy" \
    "pkg-${bin}/lib/systemd/system"

  cp "${bin}-linux-${ARCH}" "pkg-${bin}/usr/bin/${bin}"
  chmod +x "pkg-${bin}/usr/bin/${bin}"
  cp "$conf" "pkg-${bin}/etc/pertisk-proxy/${bin}.conf"
  cp "$unit" "pkg-${bin}/lib/systemd/system/${bin}.service"
}

cat > build/pertisk-proxy.service << 'SVC'
[Unit]
Description=pertisk-proxy reverse proxy
After=network.target

[Service]
Type=simple
User=pertisk-proxy
Group=pertisk-proxy
EnvironmentFile=-/etc/pertisk-proxy/pertisk-proxy.conf
ExecStart=/usr/bin/pertisk-proxy
Restart=always
RestartSec=5
LimitNOFILE=65535
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/pertisk-proxy /var/log/pertisk-proxy
PrivateTmp=true
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
SVC

cat > build/pertisk-proxy-ingress.service << 'SVC'
[Unit]
Description=pertisk-proxy Kubernetes Ingress controller
After=network.target

[Service]
Type=simple
User=pertisk-proxy
Group=pertisk-proxy
EnvironmentFile=-/etc/pertisk-proxy/pertisk-proxy-ingress.conf
ExecStart=/usr/bin/pertisk-proxy-ingress
Restart=always
RestartSec=5
LimitNOFILE=65535
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/pertisk-proxy /var/log/pertisk-proxy
PrivateTmp=true
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
SVC

cat > build/pertisk-proxy.conf << 'CONF'
# pertisk-proxy (proxy mode)
ROUTES_CONFIG=/etc/pertisk-proxy/routes.yaml
ROUTES_WATCH=true
LISTEN_HTTP=0.0.0.0:80
LISTEN_HTTPS=0.0.0.0:443
LISTEN_H3_UDP=0.0.0.0:443
ENABLE_H3=true
TLS_CERT_PATH=/etc/pertisk-proxy/tls.crt
TLS_KEY_PATH=/etc/pertisk-proxy/tls.key
PERTISK_PROXY_MODE=auto
PERTISK_LOG_LEVEL=info
CONF

cat > build/pertisk-proxy-ingress.conf << 'CONF'
# pertisk-proxy-ingress (Kubernetes Ingress controller)
INGRESS_CLASS=pertisk
WATCH_ALL_NAMESPACES=true
LISTEN_HTTP=0.0.0.0:8080
LISTEN_HTTPS=0.0.0.0:8443
LISTEN_H3_UDP=0.0.0.0:8443
ENABLE_H3=true
TLS_CERT_PATH=/etc/pertisk-proxy/tls.crt
TLS_KEY_PATH=/etc/pertisk-proxy/tls.key
PERTISK_INGRESS_MODE=auto
PERTISK_LOG_LEVEL=info
CONF

cat > preinstall.sh << 'PRE'
#!/bin/sh
set -e
if ! getent group pertisk-proxy >/dev/null 2>&1; then
  groupadd --system pertisk-proxy
fi
if ! getent passwd pertisk-proxy >/dev/null 2>&1; then
  useradd --system --gid pertisk-proxy --home-dir /var/lib/pertisk-proxy \
    --shell /usr/sbin/nologin --comment "pertisk-proxy" pertisk-proxy
fi
PRE

cat > postinstall.sh << 'POST'
#!/bin/sh
set -e
chown -R pertisk-proxy:pertisk-proxy /var/lib/pertisk-proxy /var/log/pertisk-proxy
chmod 750 /var/lib/pertisk-proxy /var/log/pertisk-proxy
command -v setcap >/dev/null 2>&1 && setcap 'cap_net_bind_service=+ep' /usr/bin/pertisk-proxy 2>/dev/null || true
command -v setcap >/dev/null 2>&1 && setcap 'cap_net_bind_service=+ep' /usr/bin/pertisk-proxy-ingress 2>/dev/null || true
command -v systemctl >/dev/null 2>&1 && systemctl daemon-reload || true
POST

chmod +x preinstall.sh postinstall.sh

for bin in "${PACKAGE_BINARIES[@]}"; do
  case "$bin" in
    pertisk-proxy)
      make_pkg_layout pertisk-proxy build/pertisk-proxy.service build/pertisk-proxy.conf "Reverse proxy with HTTP/1, HTTP/2, HTTP/3"
      ;;
    pertisk-proxy-ingress)
      make_pkg_layout pertisk-proxy-ingress build/pertisk-proxy-ingress.service build/pertisk-proxy-ingress.conf "Kubernetes Ingress controller"
      ;;
  esac

  if command -v fpm >/dev/null 2>&1; then
    fpm -s dir -t deb --force \
      -n "$bin" -v "$VERSION" -a "$deb_arch" \
      --description "pertisk-proxy ($bin)" \
      --before-install preinstall.sh --after-install postinstall.sh \
      --config-files "/etc/pertisk-proxy/${bin}.conf" \
      --deb-systemd-enable \
      -p "$RELEASE_DIR" -C "pkg-${bin}" .
  fi

  tar -czvf "$RELEASE_DIR/${bin}-v${VERSION}-linux-${ARCH}.tar.gz" \
    -C "pkg-${bin}" usr etc var lib 2>/dev/null || \
  tar -czvf "$RELEASE_DIR/${bin}-v${VERSION}-linux-${ARCH}.tar.gz" \
    -C "pkg-${bin}" usr lib
done

rm -f preinstall.sh postinstall.sh
echo "Done: release artifacts in $RELEASE_DIR/"
