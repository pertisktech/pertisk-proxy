#!/usr/bin/env bash
# Deploy tunnel RPM(s) to a remote host.
#
# Usage:
#   DEPLOY_HOST=user@host ./build/deploy-rpm-tunnel.sh
#   DEPLOY_HOST=user@host TUNNEL_PKG=server ./build/deploy-rpm-tunnel.sh
#   DEPLOY_HOST=user@host TUNNEL_PKG=client VERSION=0.1.80 ./build/deploy-rpm-tunnel.sh
#
# Env:
#   TUNNEL_PKG     — both (default), server, or client
#   PACKAGE_BUILD  — 1 build packages; 0 use existing release/*.rpm
#   PACKAGE_CLEAN  — 1 remove prior linux tunnel binaries before build
#   DEPLOY_ARCH    — auto|amd64|arm64
#   DEPLOY_HOST    — user@host

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

TUNNEL_PKG="${TUNNEL_PKG:-both}"
RAW_PACKAGE_VERSION="${1:-${PACKAGE_VERSION:-${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}}}"
VERSION="${RAW_PACKAGE_VERSION#v}"
VERSION="${VERSION#V}"
PACKAGE_CLEAN="${PACKAGE_CLEAN:-0}"
PACKAGE_BUILD="${PACKAGE_BUILD:-1}"
DEPLOY_ARCH="${DEPLOY_ARCH:-auto}"
DEPLOY_SSH_OPTS="${DEPLOY_SSH_OPTS:-}"
REMOTE_PATH="${REMOTE_PATH:-/tmp}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_HOST="${REMOTE_HOST:-}"

# shellcheck source=deploy-common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/deploy-common.sh"
resolve_deploy_host
resolve_deploy_arch

if [ -z "${REMOTE_HOST:-}" ]; then
  echo "DEPLOY_HOST=user@host is required" >&2
  exit 1
fi

case "$DEPLOY_ARCH" in
  amd64) RPM_ARCH=x86_64; PKG_ARCH=amd64 ;;
  arm64) RPM_ARCH=aarch64; PKG_ARCH=arm64 ;;
  *)
    echo "DEPLOY_ARCH must be auto, amd64, or arm64" >&2
    exit 1
    ;;
esac

case "$TUNNEL_PKG" in
  server) NAMES=(pertisk-tunnel-server) ;;
  client) NAMES=(pertisk-tunnel-client) ;;
  both)
    NAMES=(pertisk-tunnel-server pertisk-tunnel-client)
    ;;
  *)
    echo "TUNNEL_PKG must be both, server, or client" >&2
    exit 1
    ;;
esac

if [ "$PACKAGE_CLEAN" = "1" ]; then
  rm -f pertisk-tunnel-*-linux-*
fi

if [ "$PACKAGE_BUILD" = "1" ]; then
  chmod +x build/package-tunnel.sh
  case "$TUNNEL_PKG" in
    both) ./build/package-tunnel.sh "$PKG_ARCH" "$VERSION" both ;;
    server) ./build/package-tunnel.sh "$PKG_ARCH" "$VERSION" server ;;
    client) ./build/package-tunnel.sh "$PKG_ARCH" "$VERSION" client ;;
  esac
fi

ssh_cmd() {
  # shellcheck disable=SC2086
  ssh $DEPLOY_SSH_OPTS "${REMOTE_USER}@${REMOTE_HOST}" "$@"
}

scp_cmd() {
  # shellcheck disable=SC2086
  scp $DEPLOY_SSH_OPTS "$@"
}

for name in "${NAMES[@]}"; do
  RPM_FILE="${name}-${VERSION}-1.${RPM_ARCH}.rpm"
  if [ ! -f "release/${RPM_FILE}" ]; then
    alt="$(ls -1 "release/${name}-${VERSION}"*.rpm 2>/dev/null | head -n1 || true)"
    if [ -n "$alt" ]; then
      RPM_FILE="$(basename "$alt")"
    else
      echo "Missing release/${RPM_FILE}. Run with PACKAGE_BUILD=1 or build first." >&2
      exit 1
    fi
  fi
  echo "Copying release/${RPM_FILE} → ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/"
  scp_cmd "release/${RPM_FILE}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/${RPM_FILE}"
  echo "Installing ${RPM_FILE}..."
  ssh_cmd "sudo rpm -Uvh '${REMOTE_PATH}/${RPM_FILE}'"
done

echo "Tunnel RPM deploy complete."
echo "Edit /etc/pertisk-tunnel/*.toml then: sudo systemctl enable --now pertisk-tunnel-server|client"
