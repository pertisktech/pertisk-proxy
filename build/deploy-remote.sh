#!/usr/bin/env bash
# Copy DEB/RPM to a remote host and install via SSH.
#
# Usage:
#   DEPLOY_HOST=user@host ./build/deploy-remote.sh
#   DEPLOY_HOST=user@host DEPLOY_BIN=pertisk-proxy-ingress DEPLOY_PKG=rpm ./build/deploy-remote.sh
#
# Env:
#   DEPLOY_HOST     — required, e.g. root@192.168.1.10
#   DEPLOY_BIN      — pertisk-proxy (default) or pertisk-proxy-ingress
#   DEPLOY_ARCH     — amd64 (default) or arm64
#   DEPLOY_PKG      — auto (default), deb, or rpm
#   VERSION         — package version (default: git describe)
#   DEPLOY_SSH_OPTS — extra ssh options, e.g. "-i ~/.ssh/key"
#   DEPLOY_RESTART  — 1 (default) restart systemd service after install

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEPLOY_HOST="${DEPLOY_HOST:?set DEPLOY_HOST=user@host}"
DEPLOY_BIN="${DEPLOY_BIN:-pertisk-proxy}"
DEPLOY_ARCH="${DEPLOY_ARCH:-amd64}"
DEPLOY_PKG="${DEPLOY_PKG:-auto}"
DEPLOY_RESTART="${DEPLOY_RESTART:-1}"
DEPLOY_SSH_OPTS="${DEPLOY_SSH_OPTS:-}"
VERSION="${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"
VERSION="${VERSION#v}"
RELEASE_DIR="${RELEASE_DIR:-release}"

case "$DEPLOY_ARCH" in
  amd64) deb_arch=amd64; rpm_arch=x86_64 ;;
  arm64) deb_arch=arm64; rpm_arch=aarch64 ;;
  *) echo "DEPLOY_ARCH must be amd64 or arm64" >&2; exit 1 ;;
esac

ssh_cmd() {
  # shellcheck disable=SC2086
  ssh $DEPLOY_SSH_OPTS "$DEPLOY_HOST" "$@"
}

scp_cmd() {
  # shellcheck disable=SC2086
  scp $DEPLOY_SSH_OPTS "$@"
}

detect_pkg_type() {
  if [ "$DEPLOY_PKG" != "auto" ]; then
    echo "$DEPLOY_PKG"
    return
  fi
  if ssh_cmd 'command -v dpkg >/dev/null 2>&1'; then
    echo deb
  elif ssh_cmd 'command -v rpm >/dev/null 2>&1'; then
    echo rpm
  else
    echo "Cannot detect package manager on $DEPLOY_HOST (set DEPLOY_PKG=deb or rpm)" >&2
    exit 1
  fi
}

PKG_TYPE="$(detect_pkg_type)"

find_package() {
  local pattern
  case "$PKG_TYPE" in
    deb)
      pattern="${RELEASE_DIR}/${DEPLOY_BIN}_${VERSION}_${deb_arch}.deb"
      ;;
    rpm)
      pattern="${RELEASE_DIR}/${DEPLOY_BIN}-${VERSION}-1.${rpm_arch}.rpm"
      if [ ! -f "$pattern" ]; then
        pattern="$(ls -1 "${RELEASE_DIR}/${DEPLOY_BIN}"-"${VERSION}"*.rpm 2>/dev/null | head -n1 || true)"
      fi
      ;;
    *)
      echo "Unsupported DEPLOY_PKG=$PKG_TYPE" >&2
      exit 1
      ;;
  esac
  if [ -z "$pattern" ] || [ ! -f "$pattern" ]; then
    case "$DEPLOY_BIN" in
      pertisk-proxy-ingress) pkg_target=ingress ;;
      *) pkg_target=proxy ;;
    esac
    echo "Package not found for $DEPLOY_BIN $VERSION ($DEPLOY_ARCH/$PKG_TYPE)." >&2
    echo "Run: make package-${DEPLOY_ARCH} PACKAGE_TARGET=$pkg_target VERSION=$VERSION" >&2
    exit 1
  fi
  echo "$pattern"
}

PKG_FILE="$(find_package)"
REMOTE_NAME="$(basename "$PKG_FILE")"

echo "Deploying $PKG_FILE -> $DEPLOY_HOST:/tmp/$REMOTE_NAME"
scp_cmd "$PKG_FILE" "$DEPLOY_HOST:/tmp/$REMOTE_NAME"

case "$PKG_TYPE" in
  deb)
    ssh_cmd "sudo dpkg -i /tmp/$REMOTE_NAME || sudo apt-get install -f -y"
    ;;
  rpm)
    ssh_cmd "sudo rpm -Uvh /tmp/$REMOTE_NAME"
    ;;
esac

if [ "$DEPLOY_RESTART" = "1" ]; then
  ssh_cmd "sudo systemctl enable $DEPLOY_BIN --now 2>/dev/null || sudo systemctl restart $DEPLOY_BIN"
  ssh_cmd "sudo systemctl status $DEPLOY_BIN --no-pager || true"
fi

echo "Deployed $DEPLOY_BIN $VERSION to $DEPLOY_HOST"
