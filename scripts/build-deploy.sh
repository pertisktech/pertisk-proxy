#!/usr/bin/env bash
# Build all packages, then deploy RPM to DEPLOY_HOST.
#
# Usage (from repo root):
#   DEPLOY_HOST=user@proxy.example.com ./scripts/build-deploy.sh
#   DEPLOY_HOST=user@proxy.example.com VERSION=1.2.3 ./scripts/build-deploy.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "$REPO_ROOT"

export VERSION="${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"
DEPLOY_HOST="${DEPLOY_HOST:?Set DEPLOY_HOST, e.g. user@proxy.example.com}"
export DEPLOY_HOST

if [[ "$(id -u)" -eq 0 ]]; then
  make fix-perms
else
  sudo make fix-perms
fi

"${SCRIPT_DIR}/build.sh"
"${SCRIPT_DIR}/deploy.sh"
