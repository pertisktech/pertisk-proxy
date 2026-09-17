#!/usr/bin/env bash
# Publish a version to the *public* Terraform Registry namespace:
#   registry.terraform.io/pertisktech/pertisk-proxy
#
# Prerequisites:
#   - GPG secret key for signing (GPG_PASSPHRASE if the key is protected)
#   - GPG public key uploaded at https://registry.terraform.io/ → Signing Keys
#   - First time: Publish → Provider → select github.com/pertisktech/terraform-provider-pertisk-proxy
#   - gh auth'd with push access to that repo
#
# Usage (from terraform/):
#   make sync-provider-repo
#   GPG_PASSPHRASE='...' make publish-public
#   VERSION=0.1.1 GPG_PASSPHRASE='...' make publish-public
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-0.1.0}"
REPO="${PROVIDER_GITHUB_REPO:-pertisktech/terraform-provider-pertisk-proxy}"
DIST="$ROOT/dist"
TAG="v${VERSION}"

echo "==> building + signing release ${VERSION}"
VERSION="$VERSION" GPG_PASSPHRASE="${GPG_PASSPHRASE-}" bash "$ROOT/scripts/release.sh"

sig="$DIST/terraform-provider-pertisk-proxy_${VERSION}_SHA256SUMS.sig"
if [[ ! -f "$sig" ]]; then
  echo "missing $sig — signing failed" >&2
  exit 1
fi

echo "==> ensuring code is on ${REPO}"
# Sync first so the release tag points at matching source.
PROVIDER_REPO="https://github.com/${REPO}.git" bash "$ROOT/scripts/sync-provider-repo.sh"

assets=(
  "$DIST"/terraform-provider-pertisk-proxy_${VERSION}_*.zip
  "$DIST"/terraform-provider-pertisk-proxy_${VERSION}_manifest.json
  "$DIST"/terraform-provider-pertisk-proxy_${VERSION}_SHA256SUMS
  "$DIST"/terraform-provider-pertisk-proxy_${VERSION}_SHA256SUMS.sig
)

if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  echo "release ${TAG} already exists on ${REPO}; uploading/replacing assets"
  gh release upload "$TAG" "${assets[@]}" --repo "$REPO" --clobber
else
  echo "==> creating GitHub Release ${TAG} on ${REPO}"
  gh release create "$TAG" \
    --repo "$REPO" \
    --title "$TAG" \
    --notes "pertisk-proxy Terraform provider ${VERSION}

Public source: \`pertisktech/pertisk-proxy\`
" \
    "${assets[@]}"
fi

echo
echo "published GitHub Release: https://github.com/${REPO}/releases/tag/${TAG}"
echo
echo "Public Registry next:"
echo "  1. Confirm GPG key is on https://registry.terraform.io/ (Signing Keys) for namespace pertisktech"
echo "  2. First time only: Publish → Provider → ${REPO}"
echo "  3. Then use:"
echo "       source  = \"pertisktech/pertisk-proxy\""
echo "       version = \"${VERSION}\""
echo
echo "Check versions: https://registry.terraform.io/v1/providers/pertisktech/pertisk-proxy/versions"
