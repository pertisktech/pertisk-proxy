#!/usr/bin/env bash

set -euo pipefail

# Build amd64 RPM, copy to remote host, install via dnf/yum/rpm.
#
# Usage:
#   REMOTE_HOST=10.1.1.15 REMOTE_USER=root VERSION=0.5.66 ./build/deploy-rpm.sh
#   PACKAGE_TARGET=ingress REMOTE_HOST=10.1.1.15 VERSION=0.5.66 ./build/deploy-rpm.sh
#
# Env:
#   REMOTE_HOST      — remote server (default: 10.1.1.8)
#   REMOTE_USER      — SSH user (default: root)
#   VERSION          — package version (default: git describe)
#   PACKAGE_NAME     — pertisk-proxy (default) or pertisk-proxy-ingress
#   PACKAGE_TARGET   — proxy (default), ingress, or all (used when building)
#   REMOTE_PATH      — remote upload dir (default: /tmp)
#   PACKAGE_CLEAN    — 1 (default) run make package-clean before build
#   PACKAGE_BUILD    — 1 (default) build package; 0 = deploy existing release/*.rpm
#   RPM_ARCH         — x86_64 (default) or aarch64

REMOTE_HOST="${REMOTE_HOST:-10.1.1.8}"
REMOTE_USER="${REMOTE_USER:-root}"
PACKAGE_NAME="${PACKAGE_NAME:-pertisk-proxy}"
RAW_PACKAGE_VERSION="${1:-${PACKAGE_VERSION:-${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}}}"
VERSION="${RAW_PACKAGE_VERSION#v}"
VERSION="${VERSION#V}"
REMOTE_PATH="${REMOTE_PATH:-/tmp}"
PACKAGE_CLEAN="${PACKAGE_CLEAN:-1}"
PACKAGE_BUILD="${PACKAGE_BUILD:-1}"
RPM_ARCH="${RPM_ARCH:-x86_64}"

case "$PACKAGE_NAME" in
  pertisk-proxy-ingress)
    PACKAGE_TARGET="${PACKAGE_TARGET:-ingress}"
    ;;
  *)
    PACKAGE_TARGET="${PACKAGE_TARGET:-proxy}"
    PACKAGE_NAME="${PACKAGE_NAME:-pertisk-proxy}"
    ;;
esac

case "$RPM_ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
  *)
    echo "RPM_ARCH must be x86_64 or aarch64" >&2
    exit 1
    ;;
esac

RPM_FILE="${PACKAGE_NAME}-${VERSION}-1.${RPM_ARCH}.rpm"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${YELLOW}$*${NC}"; }
log_ok() { echo -e "${GREEN}$*${NC}"; }
log_err() { echo -e "${RED}$*${NC}"; }

echo -e "${GREEN}Starting RPM deployment of ${PACKAGE_NAME} version ${VERSION}${NC}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "$PACKAGE_BUILD" = "1" ]]; then
  log_info "Building RPM package (linux/${DEB_ARCH})..."
  if [[ "$PACKAGE_CLEAN" = "1" ]]; then
    log_info "Cleaning previous package binaries..."
    make package-clean
  fi
  make "package-${DEB_ARCH}" VERSION="${VERSION}" PACKAGE_TARGET="${PACKAGE_TARGET}"
fi

if [[ ! -f "release/${RPM_FILE}" ]]; then
  alt_rpm="$(ls -1 "release/${PACKAGE_NAME}-${VERSION}"*.rpm 2>/dev/null | head -n1 || true)"
  if [[ -n "$alt_rpm" ]]; then
    RPM_FILE="$(basename "$alt_rpm")"
  else
    log_err "Expected package not found: release/${RPM_FILE}"
    log_err "Check release/ contents and VERSION."
    exit 1
  fi
fi

log_info "Copying package to ${REMOTE_USER}@${REMOTE_HOST}..."
scp "release/${RPM_FILE}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/"

log_info "Installing package on remote server..."
ssh "${REMOTE_USER}@${REMOTE_HOST}" <<EOF
set -euo pipefail
PKG_PATH="${REMOTE_PATH}/${RPM_FILE}"

if command -v dnf >/dev/null 2>&1; then
  sudo dnf install -y "\${PKG_PATH}"
elif command -v yum >/dev/null 2>&1; then
  sudo yum localinstall -y "\${PKG_PATH}"
else
  sudo rpm -Uvh "\${PKG_PATH}"
fi

sudo systemctl enable "${PACKAGE_NAME}" --now
sudo systemctl restart "${PACKAGE_NAME}"
sudo systemctl is-active --quiet "${PACKAGE_NAME}"
echo "Service status:"
sudo systemctl status "${PACKAGE_NAME}" --no-pager
EOF

log_ok "RPM deployment completed successfully!"
