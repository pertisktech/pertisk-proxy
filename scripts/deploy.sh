#!/usr/bin/env bash
# Deploy pre-built pertisk-proxy packages from release/.
# Run ./scripts/build.sh first (or set PACKAGE_BUILD=1 to build during deploy).
#
# Usage (from repo root):
#   DEPLOY_HOST=user@proxy.example.com ./scripts/deploy.sh
#   DEPLOY_HOST=user@proxy.example.com PACKAGE_BUILD=1 ./scripts/deploy.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "$REPO_ROOT"

export VERSION="${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"
export PACKAGE_BUILD="${PACKAGE_BUILD:-0}"
export PACKAGE_CLEAN="${PACKAGE_CLEAN:-0}"

DEPLOY_HOST="${DEPLOY_HOST:?Set DEPLOY_HOST, e.g. user@proxy.example.com}"

deploy() {
  make "$@" VERSION="$VERSION" PACKAGE_BUILD="$PACKAGE_BUILD" PACKAGE_CLEAN="$PACKAGE_CLEAN"
}

echo "==> Deploying v${VERSION} to ${DEPLOY_HOST} (PACKAGE_BUILD=${PACKAGE_BUILD})"
deploy deploy-rpm DEPLOY_HOST="${DEPLOY_HOST}"
echo "==> Deploy complete."
