#!/usr/bin/env bash
# Build all artifacts, then deploy to configured hosts.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

export VERSION="${VERSION:-$(git describe --tags --always 2>/dev/null | sed 's/^v//' || echo '0.1.0')}"
sudo make fix-perms
"${ROOT_DIR}/build.sh"
"${ROOT_DIR}/deploy.sh"
