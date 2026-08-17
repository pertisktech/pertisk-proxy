#!/usr/bin/env bash
# Build DEB, RPM, and tarball for pertisk-proxy (proxy and/or ingress mode).
# Usage: ./build/package.sh <amd64|arm64> [VERSION] [proxy|ingress|all]
# Requires: docker. DEB/RPM use fpm (Linux) or docker/Dockerfile.package (macOS).
# Run from repo root.

set -euo pipefail

ARCH="${1:?Usage: $0 <amd64|arm64> [VERSION] [proxy|ingress|all]}"
VERSION="${2:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"
VERSION="${VERSION#v}"
TARGET="${3:-all}"
RELEASE_DIR="release"
CACHE_DIR="${CACHE_DIR:-.buildx-cache/release}"
BUILDER_NAME="${BUILDER_NAME:-pertisk-proxy-package}"
CARGO_JOBS="${PERTISK_CARGO_JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
mkdir -p "$RELEASE_DIR"

case "$ARCH" in
  amd64) deb_arch=amd64; rpm_arch=x86_64 ;;
  arm64) deb_arch=arm64; rpm_arch=aarch64 ;;
  *)
    echo "Error: ARCH must be amd64 or arm64" >&2
    exit 1
    ;;
esac

case "$TARGET" in
  proxy) PACKAGE_BINARIES=(pertisk-proxy) ;;
  ingress) PACKAGE_BINARIES=(pertisk-proxy-ingress) ;;
  all|*) PACKAGE_BINARIES=(pertisk-proxy pertisk-proxy-ingress) ;;
esac

needs_admin=0
for bin in "${PACKAGE_BINARIES[@]}"; do
  [ "$bin" = "pertisk-proxy" ] && needs_admin=1
done
if [ "$needs_admin" -eq 1 ] && [ ! -f admin/dist/index.html ]; then
  echo "Error: admin/dist not found. Run: make install-admin && make admin-dist" >&2
  exit 1
fi

HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  x86_64) HOST_ARCH=amd64 ;;
  aarch64|arm64) HOST_ARCH=arm64 ;;
esac
HOST_OS="$(uname -s)"

expected_file_pattern() {
  case "$1" in
    amd64) echo 'ELF 64-bit.*x86-64' ;;
    arm64) echo 'ELF 64-bit.*ARM aarch64' ;;
    *) return 1 ;;
  esac
}

is_valid_linux_binary() {
  local binary_path="$1"
  local arch="$2"
  local expected

  [ -f "$binary_path" ] || return 1
  command -v file >/dev/null 2>&1 || return 0

  expected="$(expected_file_pattern "$arch")" || return 1
  file "$binary_path" | grep -Eq "$expected"
}

host_rust_minor_version() {
  if ! command -v rustc >/dev/null 2>&1; then
    echo 0
    return
  fi
  rustc --version | awk '{print $2}' | cut -d. -f2
}

build_native() {
  local bin="$1"
  local features="${2:-}"
  echo "Using native cargo build for $bin (linux/$ARCH, version $VERSION)..."
  if [ -n "$features" ]; then
    CARGO_BUILD_JOBS="$CARGO_JOBS" pertisk_proxy_VERSION="$VERSION" cargo build --release --locked --bin "$bin" --features "$features"
  else
    CARGO_BUILD_JOBS="$CARGO_JOBS" pertisk_proxy_VERSION="$VERSION" cargo build --release --locked --bin "$bin"
  fi
  cp "target/release/$bin" "./${bin}-linux-${ARCH}"
}

build_binaries_docker() {
  echo "Building binaries for linux/$ARCH via Docker buildx (target=$TARGET)..."
  export DOCKER_BUILDKIT=1

  if docker buildx inspect "$BUILDER_NAME" --bootstrap >/dev/null 2>&1; then
    :
  else
    echo "Buildx builder '$BUILDER_NAME' is missing; creating..."
    docker buildx rm "$BUILDER_NAME" >/dev/null 2>&1 || true
    docker buildx create --name "$BUILDER_NAME" --driver docker-container --bootstrap
  fi

  local cache_dir="${CACHE_DIR}/${ARCH}"
  mkdir -p "$cache_dir"
  local cache_from=()
  if [ -f "${cache_dir}/index.json" ]; then
    cache_from=(--cache-from "type=local,src=${cache_dir}")
  fi
  local extract_dir
  extract_dir="$(mktemp -d)"
  local build_success=0
  for attempt in 1 2 3; do
    # ${arr[@]+"${arr[@]}"} avoids "unbound variable" under set -u when arr is empty (bash 3.2 / macOS).
    if docker buildx build --builder "$BUILDER_NAME" --platform "linux/$ARCH" \
      -f docker/Dockerfile.release \
      --target artifacts \
      ${cache_from[@]+"${cache_from[@]}"} \
      --cache-to "type=local,dest=${cache_dir},mode=max" \
      --build-arg TARGETPLATFORM="linux/$ARCH" \
      --build-arg TARGETARCH="$ARCH" \
      --build-arg PACKAGE_TARGET="$TARGET" \
      --build-arg VERSION="$VERSION" \
      --build-arg CARGO_BUILD_JOBS="$CARGO_JOBS" \
      -o "type=local,dest=${extract_dir}" .; then
      build_success=1
      break
    fi
    if [ "$attempt" -lt 3 ]; then
      echo "docker buildx build failed (attempt $attempt/3); recreating builder..."
      docker buildx rm "$BUILDER_NAME" >/dev/null 2>&1 || true
      docker buildx create --name "$BUILDER_NAME" --driver docker-container --bootstrap
    fi
  done
  if [ "$build_success" -ne 1 ]; then
    rm -rf "$extract_dir"
    echo "Error: docker buildx build failed after 3 attempts" >&2
    exit 1
  fi

  for bin in "${PACKAGE_BINARIES[@]}"; do
    if [ ! -f "${extract_dir}/${bin}" ]; then
      echo "Error: binary $bin not found in build output" >&2
      ls -la "$extract_dir" >&2 || true
      rm -rf "$extract_dir"
      exit 1
    fi
    if ! is_valid_linux_binary "${extract_dir}/${bin}" "$ARCH"; then
      echo "Error: docker build produced wrong architecture for $bin" >&2
      command -v file >/dev/null 2>&1 && file "${extract_dir}/${bin}" >&2 || true
      rm -rf "$extract_dir" "${cache_dir}"
      exit 1
    fi
    cp "${extract_dir}/${bin}" "./${bin}-linux-${ARCH}"
    chmod +x "./${bin}-linux-${ARCH}"
  done
  rm -rf "$extract_dir"
}

need_rebuild=0
for bin in "${PACKAGE_BINARIES[@]}"; do
  artifact="${bin}-linux-${ARCH}"
  version_stamp="${artifact}.version"
  if [ -f "$artifact" ] && ! is_valid_linux_binary "$artifact" "$ARCH"; then
    echo "Removing stale $artifact (not Linux/$ARCH)..."
    rm -f "$artifact" "$version_stamp"
    rm -rf "${CACHE_DIR}/${ARCH}"
  fi
  if [ ! -f "$artifact" ]; then
    need_rebuild=1
  elif [ ! -f "$version_stamp" ] || [ "$(cat "$version_stamp")" != "$VERSION" ]; then
    echo "Rebuilding $artifact (version ${VERSION}, was $(cat "$version_stamp" 2>/dev/null || echo missing))..."
    rm -f "$artifact" "$version_stamp"
    need_rebuild=1
  fi
done

if [ "$need_rebuild" -eq 1 ]; then

  if [ "$ARCH" = "$HOST_ARCH" ] && [ "$HOST_OS" = "Linux" ]; then
    HOST_RUST_MINOR="$(host_rust_minor_version)"
    if [ "$HOST_RUST_MINOR" -lt 91 ]; then
      echo "Host rustc is old; using Docker buildx..."
      build_binaries_docker
    else
      for bin in "${PACKAGE_BINARIES[@]}"; do
        artifact="${bin}-linux-${ARCH}"
        version_stamp="${artifact}.version"
        if [ ! -f "$artifact" ]; then
          case "$bin" in
            pertisk-proxy-ingress) build_native "$bin" ingress ;;
            *) build_native "$bin" ;;
          esac
          echo "$VERSION" > "$version_stamp"
        fi
      done
    fi
  else
    build_binaries_docker
  fi
  for bin in "${PACKAGE_BINARIES[@]}"; do
    artifact="${bin}-linux-${ARCH}"
    if [ -f "$artifact" ]; then
      echo "$VERSION" > "${artifact}.version"
    fi
  done
fi

for bin in "${PACKAGE_BINARIES[@]}"; do
  artifact="${bin}-linux-${ARCH}"
  if ! is_valid_linux_binary "$artifact" "$ARCH"; then
    echo "Error: $artifact is not a valid Linux/$ARCH executable" >&2
    command -v file >/dev/null 2>&1 && file "$artifact" >&2 || true
    exit 1
  fi
  cp "$artifact" "$RELEASE_DIR/"
done

# --- systemd units and config (written once) ---
cat > build/pertisk-proxy.service << 'SVC'
[Unit]
Description=pertisk-proxy reverse proxy
After=network.target
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pertisk-proxy
Group=pertisk-proxy
WorkingDirectory=/var/lib/pertisk-proxy
EnvironmentFile=-/etc/pertisk-proxy/pertisk-proxy.conf
ExecStart=/usr/bin/pertisk-proxy
Restart=always
RestartSec=5
TimeoutStopSec=30
LimitNOFILE=1048576
TasksMax=infinity
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/pertisk-proxy /var/log/pertisk-proxy
PrivateTmp=true
NoNewPrivileges=true
# Optional: pin to dedicated cores (see /usr/share/pertisk-proxy/cpu-affinity.conf.example)
# CPUAffinity=2-7

[Install]
WantedBy=multi-user.target
SVC

cat > build/pertisk-proxy-ingress.service << 'SVC'
[Unit]
Description=pertisk-proxy Kubernetes Ingress controller
After=network.target
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pertisk-proxy
Group=pertisk-proxy
WorkingDirectory=/var/lib/pertisk-proxy
EnvironmentFile=-/etc/pertisk-proxy/pertisk-proxy-ingress.conf
ExecStart=/usr/bin/pertisk-proxy-ingress
Restart=always
RestartSec=5
TimeoutStopSec=30
LimitNOFILE=1048576
TasksMax=infinity
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/pertisk-proxy /var/log/pertisk-proxy
PrivateTmp=true
NoNewPrivileges=true
# Optional: pin to dedicated cores (see /usr/share/pertisk-proxy/cpu-affinity.conf.example)
# CPUAffinity=2-7

[Install]
WantedBy=multi-user.target
SVC

cat > build/pertisk-proxy.conf << 'CONF'
# pertisk-proxy (proxy mode)
# Sites, backends, TLS, and DNS providers are stored in SQLite.
# Manage via the admin UI or PUT /api/config on the management API.

PERTISK_DB_PATH=/var/lib/pertisk-proxy/proxy.sqlite

LISTEN_HTTP=[::]:80
LISTEN_HTTPS=[::]:443
LISTEN_H3_UDP=[::]:443
ENABLE_H3=true

# Admin UI + management API (bind locally; put a reverse proxy in front in production)
PERTISK_MANAGEMENT_ADDR=[::]:9080
# PERTISK_ADMIN_PASSWORD=change-me

# Optional one-time import when the database is empty (legacy routes.yaml)
# ROUTES_CONFIG=/etc/pertisk-proxy/routes.yaml

# Optional global fallback TLS when no per-site certs exist in the database:
# TLS_CERT_PATH=/etc/pertisk-proxy/tls.crt
# TLS_KEY_PATH=/etc/pertisk-proxy/tls.key

PERTISK_PROXY_MODE=auto
PERTISK_LOG_LEVEL=info

# Downstream (client) TCP keepalive on HTTP/HTTPS listeners (default: 60/10/5)
# PERTISK_TCP_KEEPALIVE=0
# PERTISK_TCP_KEEPALIVE_IDLE_SECS=60
# PERTISK_TCP_KEEPALIVE_INTERVAL_SECS=10
# PERTISK_TCP_KEEPALIVE_COUNT=5

# H3 → upstream connection pool (defaults scale with performance mode)
# PERTISK_H3_UPSTREAM_POOL_MAX_IDLE=256
# PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS=120
# PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS=60

# ACME (Let's Encrypt)
# PERTISK_ACME_STAGING=true
CONF

cat > build/pertisk-proxy-ingress.conf << 'CONF'
# pertisk-proxy-ingress (Kubernetes Ingress controller, systemd/DEB/RPM deploy)
INGRESS_CLASS=pertisk
WATCH_ALL_NAMESPACES=true
LISTEN_HTTP=0.0.0.0:8080
LISTEN_HTTPS=0.0.0.0:8443
LISTEN_H3_UDP=[::]:8443
ENABLE_H3=true
TLS_CERT_PATH=/etc/pertisk-proxy/tls.crt
TLS_KEY_PATH=/etc/pertisk-proxy/tls.key
PERTISK_INGRESS_MODE=performance
PERTISK_LOG_LEVEL=info
# H3 → upstream pool (optional overrides; performance mode defaults to 256 idle/host)
# PERTISK_H3_UPSTREAM_POOL_MAX_IDLE=256
# Required when running outside the cluster (DEB/RPM on a VM/bare-metal node):
# KUBECONFIG=/etc/pertisk-proxy/kubeconfig
CONF

make_pkg_layout() {
  local bin="$1"
  local unit="$2"
  local conf="$3"

  rm -rf "pkg-${bin}"
  mkdir -p "pkg-${bin}/usr/bin" \
    "pkg-${bin}/etc/pertisk-proxy" \
    "pkg-${bin}/etc/sysctl.d" \
    "pkg-${bin}/usr/share/pertisk-proxy" \
    "pkg-${bin}/var/lib/pertisk-proxy" \
    "pkg-${bin}/var/log/pertisk-proxy" \
    "pkg-${bin}/lib/systemd/system"

  cp "${bin}-linux-${ARCH}" "pkg-${bin}/usr/bin/${bin}"
  chmod +x "pkg-${bin}/usr/bin/${bin}"
  cp "$conf" "pkg-${bin}/etc/pertisk-proxy/${bin}.conf"
  cp "$unit" "pkg-${bin}/lib/systemd/system/${bin}.service"
  cp build/99-pertisk-proxy.conf "pkg-${bin}/etc/sysctl.d/99-pertisk-proxy.conf"
  cp build/cpu-affinity.conf.example "pkg-${bin}/usr/share/pertisk-proxy/cpu-affinity.conf.example"

  if [ "$bin" = "pertisk-proxy" ] && [ -d admin/dist ]; then
    mkdir -p "pkg-${bin}/usr/share/pertisk-proxy/admin"
    cp -r admin/dist "pkg-${bin}/usr/share/pertisk-proxy/admin/"
  fi

  if [ "$bin" = "pertisk-proxy-ingress" ]; then
    mkdir -p "pkg-${bin}/etc/pertisk-proxy/kubernetes"
    if [ -f deploy/kubernetes-rbac.yaml ]; then
      cp deploy/kubernetes-rbac.yaml "pkg-${bin}/etc/pertisk-proxy/kubernetes/rbac.yaml"
    fi
    if [ -f deploy/example-ingress.yaml ]; then
      cp deploy/example-ingress.yaml "pkg-${bin}/etc/pertisk-proxy/kubernetes/example-ingress.yaml"
    fi
  fi
}

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
mkdir -p /var/lib/pertisk-proxy/certs
chown -R pertisk-proxy:pertisk-proxy /var/lib/pertisk-proxy /var/log/pertisk-proxy
chmod 750 /var/lib/pertisk-proxy /var/log/pertisk-proxy
chmod 660 /var/lib/pertisk-proxy/*.sqlite 2>/dev/null || true
for conf in /etc/pertisk-proxy/pertisk-proxy.conf; do
  if [ -f "$conf" ] && ! grep -q '^PERTISK_DB_PATH=' "$conf"; then
    printf '\n# Added by package postinstall\nPERTISK_DB_PATH=/var/lib/pertisk-proxy/proxy.sqlite\n' >> "$conf"
  fi
done
if [ -d /etc/pertisk-proxy ]; then
  chown -R root:pertisk-proxy /etc/pertisk-proxy
  chmod 750 /etc/pertisk-proxy
  chmod 644 /etc/pertisk-proxy/*.crt /etc/pertisk-proxy/*.pem 2>/dev/null || true
  chmod 640 /etc/pertisk-proxy/*.key 2>/dev/null || true
fi
for bin in pertisk-proxy pertisk-proxy-ingress; do
  if [ -x "/usr/bin/$bin" ]; then
    command -v setcap >/dev/null 2>&1 && setcap 'cap_net_bind_service=+ep' "/usr/bin/$bin" 2>/dev/null || true
  fi
done
# Apply kernel buffer / backlog ceilings so QUIC SO_RCVBUF requests are not clamped.
if [ -f /etc/sysctl.d/99-pertisk-proxy.conf ]; then
  if command -v sysctl >/dev/null 2>&1; then
    sysctl -p /etc/sysctl.d/99-pertisk-proxy.conf >/dev/null 2>&1 || true
  fi
fi
command -v systemctl >/dev/null 2>&1 && systemctl daemon-reload || true
POST

cat > preremove.sh << 'PRE'
#!/bin/sh
set -e
for svc in pertisk-proxy pertisk-proxy-ingress; do
  if command -v systemctl >/dev/null 2>&1; then
    systemctl stop "$svc" 2>/dev/null || true
    systemctl disable "$svc" 2>/dev/null || true
  fi
done
PRE

chmod +x preinstall.sh postinstall.sh preremove.sh

for bin in "${PACKAGE_BINARIES[@]}"; do
  case "$bin" in
    pertisk-proxy)
      make_pkg_layout pertisk-proxy build/pertisk-proxy.service build/pertisk-proxy.conf
      ;;
    pertisk-proxy-ingress)
      make_pkg_layout pertisk-proxy-ingress build/pertisk-proxy-ingress.service build/pertisk-proxy-ingress.conf
      ;;
  esac
done

if command -v xattr >/dev/null 2>&1; then
  for bin in "${PACKAGE_BINARIES[@]}"; do
    xattr -cr "pkg-${bin}" 2>/dev/null || true
  done
fi

[ "$(uname -s)" = "Darwin" ] && export COPYFILE_DISABLE=1

BUILT_DEB_RPM=false
if [ "$(uname -s)" = "Darwin" ]; then
  echo "Building DEB and RPM in Linux container..."
  docker build --network=host -q -f docker/Dockerfile.package -t pertisk-proxy-package .
  docker run --rm \
    -v "$(pwd):/work" -w /work \
    -e PACKAGE_BINARIES="${PACKAGE_BINARIES[*]}" \
    -e VERSION="$VERSION" \
    -e deb_arch="$deb_arch" \
    -e rpm_arch="$rpm_arch" \
    pertisk-proxy-package bash /work/build/deb-rpm.sh
  BUILT_DEB_RPM=true
else
  FPM_CMD=""
  if command -v fpm >/dev/null 2>&1; then
    FPM_CMD="fpm"
  else
    for dir in "$HOME/.gem/ruby/"*/bin "$HOME/.local/share/gem/ruby/"*/bin; do
      [ -x "${dir}/fpm" ] 2>/dev/null || continue
      FPM_CMD="${dir}/fpm"
      break
    done
  fi

  if [ -n "$FPM_CMD" ]; then
    for bin in "${PACKAGE_BINARIES[@]}"; do
      $FPM_CMD -s dir -t deb --force \
        -n "$bin" -v "$VERSION" -a "$deb_arch" \
        --description "pertisk-proxy ($bin)" \
        --url "https://github.com/pertisktech/pertisk-proxy" \
        --maintainer "Pertisk Team" --license "Apache-2.0" --vendor "Pertisk" \
        --category "net" --depends libcap2-bin \
        --before-install preinstall.sh --after-install postinstall.sh --before-remove preremove.sh \
        --config-files "/etc/pertisk-proxy/${bin}.conf" \
        --directories /var/lib/pertisk-proxy --directories /var/log/pertisk-proxy \
        --deb-systemd-enable --deb-no-default-config-files \
        -p "$RELEASE_DIR" -C "pkg-${bin}" .

      if command -v rpmbuild >/dev/null 2>&1; then
        $FPM_CMD -s dir -t rpm --force \
          -n "$bin" -v "$VERSION" -a "$rpm_arch" \
          --description "pertisk-proxy ($bin)" \
          --url "https://github.com/pertisktech/pertisk-proxy" \
          --maintainer "Pertisk Team" --license "Apache-2.0" --vendor "Pertisk" \
          --category "System Environment/Daemons" \
          --depends libcap --depends shadow-utils \
          --before-install preinstall.sh --after-install postinstall.sh --before-remove preremove.sh \
          --config-files "/etc/pertisk-proxy/${bin}.conf" \
          --directories /var/lib/pertisk-proxy --directories /var/log/pertisk-proxy \
          --rpm-os linux \
          -p "$RELEASE_DIR" -C "pkg-${bin}" .
      fi
    done
    BUILT_DEB_RPM=true
  else
    echo "fpm not found: skipping DEB/RPM (gem install fpm --user-install)"
  fi
fi

for bin in "${PACKAGE_BINARIES[@]}"; do
  tar -czvf "$RELEASE_DIR/${bin}-v${VERSION}-linux-${ARCH}.tar.gz" \
    -C "pkg-${bin}" usr etc var lib 2>/dev/null || \
  tar -czvf "$RELEASE_DIR/${bin}-v${VERSION}-linux-${ARCH}.tar.gz" \
    -C "pkg-${bin}" usr lib
done

rm -f preinstall.sh postinstall.sh preremove.sh

echo "Done. Artifacts in $RELEASE_DIR/:"
ls -1 "$RELEASE_DIR"/*"${ARCH}"* 2>/dev/null || ls -1 "$RELEASE_DIR/"
