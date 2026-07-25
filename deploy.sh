#!/usr/bin/env bash
# Deploy pre-built pertisk-proxy packages from release/.
# Run ./build.sh first (or set PACKAGE_BUILD=1 to build during deploy).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

export VERSION="${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"
export PACKAGE_BUILD="${PACKAGE_BUILD:-0}"
export PACKAGE_CLEAN="${PACKAGE_CLEAN:-0}"

deploy() {
  make "$@" VERSION="$VERSION" PACKAGE_BUILD="$PACKAGE_BUILD" PACKAGE_CLEAN="$PACKAGE_CLEAN"
}

echo "==> Deploying v${VERSION} (PACKAGE_BUILD=${PACKAGE_BUILD})"

# --- pertisk-proxy (RPM) ---
deploy deploy-rpm DEPLOY_HOST=nat@103.117.150.228
deploy deploy-rpm DEPLOY_HOST=root@135.181.197.40
#deploy deploy-rpm DEPLOY_HOST=rocky@10.1.1.12
deploy deploy-rpm DEPLOY_HOST=root@187.77.155.197
#deploy deploy-rpm-arm64 DEPLOY_HOST=almalinux@10.1.1.233
#deploy deploy-rpm DEPLOY_HOST=almalinux@10.1.1.13
deploy deploy-rpm DEPLOY_HOST=almalinux@10.1.1.20
# deploy deploy-rpm-arm DEPLOY_HOST=root@157.180.22.221
echo "==> Deploy complete."
