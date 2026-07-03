#!/usr/bin/env bash

set -euo pipefail

# Build RPM, copy to remote host, install via dnf/yum/rpm.
#
# Usage:
#   DEPLOY_HOST=user@host VERSION=0.5.66 ./build/deploy-rpm.sh
#   PACKAGE_TARGET=ingress DEPLOY_HOST=user@host VERSION=0.5.66 ./build/deploy-rpm.sh
#
# Env:
#   REMOTE_HOST      — remote server
#   REMOTE_USER      — SSH user (default: root)
#   VERSION          — package version (default: git describe)
#   PACKAGE_NAME     — pertisk-proxy (default) or pertisk-proxy-ingress
#   PACKAGE_TARGET   — proxy (default), ingress, or all (used when building)
#   REMOTE_PATH      — remote upload dir (default: /tmp)
#   PACKAGE_CLEAN    — 1 (default) run make package-clean before build
#   PACKAGE_BUILD    — 1 (default) build package; 0 = deploy existing release/*.rpm
#   DEPLOY_ARCH      — auto (default), amd64, or arm64 (auto = detect via SSH)
#   RPM_ARCH         — x86_64 or aarch64 (derived from DEPLOY_ARCH unless set)
#   DEPLOY_HOST      — user@host (overrides REMOTE_USER + REMOTE_HOST)
#   DEPLOY_SSH_OPTS  — extra ssh/scp options

REMOTE_HOST="${REMOTE_HOST:-}"
REMOTE_USER="${REMOTE_USER:-root}"
PACKAGE_NAME="${PACKAGE_NAME:-pertisk-proxy}"
RAW_PACKAGE_VERSION="${1:-${PACKAGE_VERSION:-${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}}}"
VERSION="${RAW_PACKAGE_VERSION#v}"
VERSION="${VERSION#V}"
REMOTE_PATH="${REMOTE_PATH:-/tmp}"
PACKAGE_CLEAN="${PACKAGE_CLEAN:-1}"
PACKAGE_BUILD="${PACKAGE_BUILD:-1}"
DEPLOY_ARCH="${DEPLOY_ARCH:-auto}"
DEPLOY_SSH_OPTS="${DEPLOY_SSH_OPTS:-}"
RPM_ARCH="${RPM_ARCH:-}"

# shellcheck source=deploy-common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/deploy-common.sh"
resolve_deploy_host
resolve_deploy_arch

if [ -z "${REMOTE_HOST:-}" ]; then
  echo "REMOTE_HOST or DEPLOY_HOST is required. Usage: DEPLOY_HOST=user@host ./build/deploy-rpm.sh" >&2
  exit 1
fi

REMOTE_HOST="${REMOTE_HOST}"

case "$PACKAGE_NAME" in
  pertisk-proxy-ingress)
    PACKAGE_TARGET="${PACKAGE_TARGET:-ingress}"
    ;;
  *)
    PACKAGE_TARGET="${PACKAGE_TARGET:-proxy}"
    PACKAGE_NAME="${PACKAGE_NAME:-pertisk-proxy}"
    ;;
esac

if [ -z "$RPM_ARCH" ]; then
  case "$DEPLOY_ARCH" in
    amd64) RPM_ARCH=x86_64 ;;
    arm64) RPM_ARCH=aarch64 ;;
    *)
      echo "DEPLOY_ARCH must be auto, amd64, or arm64 (got: $DEPLOY_ARCH)" >&2
      exit 1
      ;;
  esac
fi

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
# shellcheck disable=SC2086
scp $DEPLOY_SSH_OPTS "release/${RPM_FILE}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/"

log_info "Installing package on remote server..."
# shellcheck disable=SC2086
ssh $DEPLOY_SSH_OPTS "${REMOTE_USER}@${REMOTE_HOST}" <<EOF
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
