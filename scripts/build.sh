#!/usr/bin/env bash
# Build pertisk-proxy packages (DEB + RPM) for amd64 + arm64.
#
# Usage (from repo root):
#   ./scripts/build.sh
#   VERSION=1.2.3 ./scripts/build.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "$REPO_ROOT"

export VERSION="${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"

mkdir -p release

echo "==> Building pertisk-proxy packages (DEB + RPM, amd64 + arm64) v${VERSION}"
make package-clean
make package VERSION="$VERSION"

docker system prune -f
echo "==> Build complete. Artifacts in release/"
