#!/usr/bin/env bash
# Build DEB/RPM for pertisk-tunnel-server and pertisk-tunnel-client.
#
# Usage (from repo root):
#   ./build/package-tunnel.sh amd64 [VERSION]
#   ./build/package-tunnel.sh arm64 [VERSION]
#   ./build/package-tunnel.sh amd64 0.1.80 both   # default both
#   ./build/package-tunnel.sh amd64 0.1.80 server
#   ./build/package-tunnel.sh amd64 0.1.80 client
#
# Artifacts land in release/:
#   pertisk-tunnel-server-<ver>-1.x86_64.rpm
#   pertisk-tunnel-client-<ver>-1.x86_64.rpm
#   (+ .deb)
#
# CI: prefer host cargo / cargo-zigbuild (set FORCE_DOCKER_TUNNEL=1 only if needed).

set -euo pipefail

ARCH="${1:?Usage: $0 <amd64|arm64> [VERSION] [both|server|client]}"
VERSION="${2:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"
VERSION="${VERSION#v}"
TARGET="${3:-both}"
RELEASE_DIR="release"
CACHE_DIR="${CACHE_DIR:-.buildx-cache/tunnel}"
BUILDER_NAME="${BUILDER_NAME:-pertisk-proxy-package}"
CARGO_JOBS="${PERTISK_CARGO_JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
mkdir -p "$RELEASE_DIR"

case "$ARCH" in
  amd64) deb_arch=amd64; rpm_arch=x86_64; RUST_TARGET=x86_64-unknown-linux-gnu ;;
  arm64) deb_arch=arm64; rpm_arch=aarch64; RUST_TARGET=aarch64-unknown-linux-gnu ;;
  *)
    echo "Error: ARCH must be amd64 or arm64" >&2
    exit 1
    ;;
esac

case "$TARGET" in
  server) PACKAGE_BINARIES=(pertisk-tunnel-server) ;;
  client) PACKAGE_BINARIES=(pertisk-tunnel-client) ;;
  both|*) PACKAGE_BINARIES=(pertisk-tunnel-server pertisk-tunnel-client) ;;
esac

HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  x86_64) HOST_ARCH=amd64 ;;
  aarch64|arm64) HOST_ARCH=arm64 ;;
esac
HOST_OS="$(uname -s)"

is_valid_linux_binary() {
  local binary_path="$1"
  local arch="$2"
  [ -f "$binary_path" ] || return 1
  command -v file >/dev/null 2>&1 || return 0
  case "$arch" in
    amd64) file "$binary_path" | grep -Eq 'ELF 64-bit.*x86-64' ;;
    arm64) file "$binary_path" | grep -Eq 'ELF 64-bit.*ARM aarch64' ;;
    *) return 1 ;;
  esac
}

build_native() {
  echo "Native cargo build for tunnel (linux/$ARCH)..."
  CARGO_BUILD_JOBS="$CARGO_JOBS" cargo build --release --locked \
    -p pertisk-tunnel-server -p pertisk-tunnel-client
  cp target/release/pertisk-tunnel-server "./pertisk-tunnel-server-linux-${ARCH}"
  cp target/release/pertisk-tunnel-client "./pertisk-tunnel-client-linux-${ARCH}"
}

build_zig() {
  echo "Cross-compiling tunnel for linux/$ARCH via cargo-zigbuild (${RUST_TARGET})..."
  chmod +x build/ci-ensure-zig.sh
  build/ci-ensure-zig.sh
  case "$(uname -m)" in
    x86_64|amd64) _za=x86_64 ;;
    *) _za=aarch64 ;;
  esac
  _zd="${HOME}/.local/zig/zig-linux-${_za}-${ZIG_VERSION:-0.13.0}"
  if [ -x "${_zd}/zig" ]; then
    export PATH="${_zd}:${PATH}"
  fi
  if [ -d "${HOME}/.cargo/bin" ]; then
    export PATH="${HOME}/.cargo/bin:${PATH}"
  fi

  rustup target add "$RUST_TARGET"
  CARGO_BUILD_JOBS="$CARGO_JOBS" cargo zigbuild --release --locked \
    --target "$RUST_TARGET" \
    -p pertisk-tunnel-server -p pertisk-tunnel-client
  cp "target/${RUST_TARGET}/release/pertisk-tunnel-server" "./pertisk-tunnel-server-linux-${ARCH}"
  cp "target/${RUST_TARGET}/release/pertisk-tunnel-client" "./pertisk-tunnel-client-linux-${ARCH}"
}

build_docker() {
  echo "Building tunnel binaries for linux/$ARCH via Docker buildx..."
  export DOCKER_BUILDKIT=1
  if ! docker buildx inspect "$BUILDER_NAME" --bootstrap >/dev/null 2>&1; then
    docker buildx rm "$BUILDER_NAME" >/dev/null 2>&1 || true
    docker buildx create --name "$BUILDER_NAME" --driver docker-container \
      --driver-opt network=host --bootstrap
  fi
  local cache_dir="${CACHE_DIR}/${ARCH}"
  mkdir -p "$cache_dir"
  local cache_from=()
  if [ -f "${cache_dir}/index.json" ]; then
    cache_from=(--cache-from "type=local,src=${cache_dir}")
  fi
  local extract_dir
  extract_dir="$(mktemp -d)"
  docker buildx build --builder "$BUILDER_NAME" --platform "linux/$ARCH" --network=host \
    -f docker/Dockerfile.tunnel \
    --target artifacts \
    ${cache_from[@]+"${cache_from[@]}"} \
    --cache-to "type=local,dest=${cache_dir},mode=max" \
    --build-arg TARGETPLATFORM="linux/$ARCH" \
    --build-arg TARGETARCH="$ARCH" \
    --build-arg BUILDARCH="$HOST_ARCH" \
    --build-arg VERSION="$VERSION" \
    --build-arg CARGO_BUILD_JOBS="$CARGO_JOBS" \
    -o "type=local,dest=${extract_dir}" \
    .
  cp "${extract_dir}/pertisk-tunnel-server" "./pertisk-tunnel-server-linux-${ARCH}"
  cp "${extract_dir}/pertisk-tunnel-client" "./pertisk-tunnel-client-linux-${ARCH}"
  rm -rf "$extract_dir"
}

need_build=0
for bin in "${PACKAGE_BINARIES[@]}"; do
  if ! is_valid_linux_binary "./${bin}-linux-${ARCH}" "$ARCH"; then
    need_build=1
  fi
done

if [ "$need_build" -eq 1 ]; then
  if [ "${FORCE_DOCKER_TUNNEL:-0}" = "1" ]; then
    build_docker
  elif [ "$HOST_OS" = "Linux" ] && [ "$HOST_ARCH" = "$ARCH" ]; then
    build_native
  elif [ "$HOST_OS" = "Linux" ] && command -v cargo >/dev/null 2>&1; then
    # Cross on host (avoids Docker Hub pulls / QEMU).
    if ! build_zig; then
      echo "cargo-zigbuild failed; falling back to Docker..." >&2
      build_docker
    fi
  else
    build_docker
  fi
else
  echo "Reusing existing tunnel Linux binaries for $ARCH"
fi

for bin in "${PACKAGE_BINARIES[@]}"; do
  if ! is_valid_linux_binary "./${bin}-linux-${ARCH}" "$ARCH"; then
    echo "Error: missing or invalid binary ${bin}-linux-${ARCH}" >&2
    exit 1
  fi
done

# --- package layout ---
make_tunnel_pkg() {
  local bin="$1"
  local root="pkg-${bin}"
  rm -rf "$root"
  mkdir -p \
    "$root/usr/bin" \
    "$root/etc/pertisk-tunnel" \
    "$root/usr/lib/systemd/system" \
    "$root/usr/share/doc/${bin}" \
    "$root/var/lib/pertisk-tunnel"
  chmod 750 "$root/var/lib/pertisk-tunnel"
  touch "$root/var/lib/pertisk-tunnel/.keep"

  cp "./${bin}-linux-${ARCH}" "$root/usr/bin/${bin}"
  chmod 755 "$root/usr/bin/${bin}"

  if [ "$bin" = "pertisk-tunnel-server" ]; then
    cp tunnel/examples/server.toml "$root/etc/pertisk-tunnel/server.toml"
    cp tunnel/examples/pertisk-tunnel-server.service \
      "$root/usr/lib/systemd/system/pertisk-tunnel-server.service"
  else
    cp tunnel/examples/client.toml "$root/etc/pertisk-tunnel/client.toml"
    cp tunnel/examples/pertisk-tunnel-client.service \
      "$root/usr/lib/systemd/system/pertisk-tunnel-client.service"
    # System unit for client: use /etc path (not %h) for RPM installs.
    sed -i.bak 's|ExecStart=.*|ExecStart=/usr/bin/pertisk-tunnel-client --config /etc/pertisk-tunnel/client.toml|' \
      "$root/usr/lib/systemd/system/pertisk-tunnel-client.service"
    rm -f "$root/usr/lib/systemd/system/pertisk-tunnel-client.service.bak"
  fi

  cp docs/tunnel.md "$root/usr/share/doc/${bin}/tunnel.md"
  cp tunnel/README.md "$root/usr/share/doc/${bin}/README.md" 2>/dev/null || true
}

cat > preinstall-tunnel.sh << 'PRE'
#!/bin/sh
set -e
if ! getent group pertisk-tunnel >/dev/null 2>&1; then
  groupadd --system pertisk-tunnel
fi
if ! getent passwd pertisk-tunnel >/dev/null 2>&1; then
  useradd --system --gid pertisk-tunnel --home-dir /var/lib/pertisk-tunnel \
    --shell /usr/sbin/nologin --comment "pertisk-tunnel" pertisk-tunnel
fi
mkdir -p /var/lib/pertisk-tunnel
chown pertisk-tunnel:pertisk-tunnel /var/lib/pertisk-tunnel
chmod 750 /var/lib/pertisk-tunnel
PRE

cat > postinstall-tunnel.sh << 'POST'
#!/bin/sh
set -e
if [ -d /etc/pertisk-tunnel ]; then
  chown -R root:pertisk-tunnel /etc/pertisk-tunnel
  chmod 750 /etc/pertisk-tunnel
  chmod 640 /etc/pertisk-tunnel/*.toml 2>/dev/null || true
fi
command -v systemctl >/dev/null 2>&1 && systemctl daemon-reload || true
echo "pertisk-tunnel: edit /etc/pertisk-tunnel/*.toml then: systemctl enable --now pertisk-tunnel-server|client"
POST

cat > preremove-tunnel.sh << 'PRE'
#!/bin/sh
set -e
for svc in pertisk-tunnel-server pertisk-tunnel-client; do
  if command -v systemctl >/dev/null 2>&1; then
    systemctl stop "$svc" 2>/dev/null || true
    systemctl disable "$svc" 2>/dev/null || true
  fi
done
PRE

chmod +x preinstall-tunnel.sh postinstall-tunnel.sh preremove-tunnel.sh

for bin in "${PACKAGE_BINARIES[@]}"; do
  make_tunnel_pkg "$bin"
done

if command -v xattr >/dev/null 2>&1; then
  for bin in "${PACKAGE_BINARIES[@]}"; do
    xattr -cr "pkg-${bin}" 2>/dev/null || true
  done
fi
[ "$(uname -s)" = "Darwin" ] && export COPYFILE_DISABLE=1

run_fpm() {
  local bin="$1"
  local fmt="$2"
  local arch="$3"
  local extra=()
  if [ "$fmt" = "deb" ]; then
    extra+=(--category net --deb-systemd-enable --deb-no-default-config-files)
  else
    extra+=(--category "System Environment/Daemons" --rpm-os linux --depends shadow-utils)
  fi
  local conf
  if [ "$bin" = "pertisk-tunnel-server" ]; then
    conf="/etc/pertisk-tunnel/server.toml"
  else
    conf="/etc/pertisk-tunnel/client.toml"
  fi
  fpm -s dir -t "$fmt" --force \
    -n "$bin" \
    -v "$VERSION" \
    -a "$arch" \
    --description "pertisk reverse tunnel ($bin)" \
    --url "https://github.com/pertisktech/pertisk-proxy" \
    --maintainer "Pertisk Team" \
    --license "Apache-2.0" \
    --vendor "Pertisk" \
    --before-install preinstall-tunnel.sh \
    --after-install postinstall-tunnel.sh \
    --before-remove preremove-tunnel.sh \
    --config-files "$conf" \
    --directories /etc/pertisk-tunnel \
    --directories /var/lib/pertisk-tunnel \
    "${extra[@]}" \
    -p "$RELEASE_DIR" \
    -C "pkg-${bin}" \
    .
}

if [ "${FORCE_DOCKER_FPM:-0}" = "1" ] \
  || [ "$(uname -s)" = "Darwin" ] \
  || ! command -v fpm >/dev/null 2>&1 \
  || { [ "$(uname -s)" = "Linux" ] && ! command -v rpmbuild >/dev/null 2>&1; }; then
  echo "Building tunnel DEB/RPM in Linux container..."
  docker build --network=host -f docker/Dockerfile.package -t pertisk-proxy-package .
  # Write helper used inside container
  cat > build/deb-rpm-tunnel.sh << 'INNER'
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
INNER
  chmod +x build/deb-rpm-tunnel.sh
  docker run --rm --network=host \
    -v "$(pwd):/work" -w /work \
    -e PACKAGE_BINARIES="${PACKAGE_BINARIES[*]}" \
    -e VERSION="$VERSION" \
    -e deb_arch="$deb_arch" \
    -e rpm_arch="$rpm_arch" \
    pertisk-proxy-package bash /work/build/deb-rpm-tunnel.sh
else
  for bin in "${PACKAGE_BINARIES[@]}"; do
    run_fpm "$bin" deb "$deb_arch"
    run_fpm "$bin" rpm "$rpm_arch"
  done
fi

echo "==> Tunnel packages in ${RELEASE_DIR}/:"
ls -la "$RELEASE_DIR"/pertisk-tunnel-*.{rpm,deb} 2>/dev/null || ls -la "$RELEASE_DIR"/pertisk-tunnel-* || true
