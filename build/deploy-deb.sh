#!/usr/bin/env bash

set -euo pipefail

# Build amd64 DEB, copy to remote host, install via dpkg.
#
# Usage:
#   REMOTE_HOST=10.1.1.15 REMOTE_USER=root VERSION=0.5.66 ./build/deploy-deb.sh
#   PACKAGE_TARGET=ingress REMOTE_HOST=10.1.1.15 VERSION=0.5.66 ./build/deploy-deb.sh
#
# Env:
#   REMOTE_HOST      — remote server (default: 10.1.1.8)
#   REMOTE_USER      — SSH user (default: root)
#   VERSION          — package version (default: git describe)
#   PACKAGE_NAME     — pertisk-proxy (default) or pertisk-proxy-ingress
#   PACKAGE_TARGET   — proxy (default), ingress, or all (used when building)
#   REMOTE_PATH      — remote upload dir (default: /tmp)
#   PACKAGE_CLEAN    — 1 (default) run make package-clean before build
#   PACKAGE_BUILD    — 1 (default) build package; 0 = deploy existing release/*.deb
#   DEB_ARCH         — amd64 (default) or arm64

REMOTE_HOST="${REMOTE_HOST:-10.1.1.8}"
REMOTE_USER="${REMOTE_USER:-root}"
PACKAGE_NAME="${PACKAGE_NAME:-pertisk-proxy}"
RAW_PACKAGE_VERSION="${1:-${PACKAGE_VERSION:-${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}}}"
VERSION="${RAW_PACKAGE_VERSION#v}"
VERSION="${VERSION#V}"
REMOTE_PATH="${REMOTE_PATH:-/tmp}"
PACKAGE_CLEAN="${PACKAGE_CLEAN:-1}"
PACKAGE_BUILD="${PACKAGE_BUILD:-1}"
DEB_ARCH="${DEB_ARCH:-amd64}"

case "$PACKAGE_NAME" in
  pertisk-proxy-ingress)
    PACKAGE_TARGET="${PACKAGE_TARGET:-ingress}"
    ;;
  *)
    PACKAGE_TARGET="${PACKAGE_TARGET:-proxy}"
    PACKAGE_NAME="${PACKAGE_NAME:-pertisk-proxy}"
    ;;
esac

DEB_FILE="${PACKAGE_NAME}_${VERSION}_${DEB_ARCH}.deb"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${YELLOW}$*${NC}"; }
log_ok() { echo -e "${GREEN}$*${NC}"; }
log_err() { echo -e "${RED}$*${NC}"; }

echo -e "${GREEN}Starting Debian deployment of ${PACKAGE_NAME} version ${VERSION}${NC}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "$PACKAGE_BUILD" = "1" ]]; then
  log_info "Building Debian package (linux/${DEB_ARCH})..."
  if [[ "$PACKAGE_CLEAN" = "1" ]]; then
    log_info "Cleaning previous package binaries..."
    make package-clean
  fi
  make "package-${DEB_ARCH}" VERSION="${VERSION}" PACKAGE_TARGET="${PACKAGE_TARGET}"
fi

if [[ ! -f "release/${DEB_FILE}" ]]; then
  log_err "Expected package not found: release/${DEB_FILE}"
  log_err "Check release/ contents and VERSION."
  exit 1
fi

log_info "Copying package to ${REMOTE_USER}@${REMOTE_HOST}..."
scp "release/${DEB_FILE}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/"

log_info "Installing package on remote server..."
ssh "${REMOTE_USER}@${REMOTE_HOST}" <<EOF
set -euo pipefail
PKG_PATH="${REMOTE_PATH}/${DEB_FILE}"

sudo dpkg -i "\${PKG_PATH}"
sudo env DEBIAN_FRONTEND=noninteractive NEEDRESTART_SUSPEND=1 apt-get -f install -y

sudo systemctl enable "${PACKAGE_NAME}" --now
sudo systemctl restart "${PACKAGE_NAME}"
sudo systemctl is-active --quiet "${PACKAGE_NAME}"
echo "Service status:"
sudo systemctl status "${PACKAGE_NAME}" --no-pager
EOF

log_ok "Debian deployment completed successfully!"
