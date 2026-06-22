#!/usr/bin/env bash
# Create or update a GitHub Release and upload DEB/RPM/tarball assets.
# Uses gh CLI so the release body is always replaced (softprops often skips body updates).
set -euo pipefail

: "${VERSION:?VERSION required}"
: "${TAG:?TAG required}"
: "${PACKAGES_DIR:?PACKAGES_DIR required}"
: "${NOTES_FILE:?NOTES_FILE required}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN required}"

export GH_TOKEN="$GITHUB_TOKEN"

# shellcheck source=ci-install-gh.sh
source "$(dirname "$0")/ci-install-gh.sh"
ensure_gh

if ! command -v gh >/dev/null 2>&1; then
  echo "::error::GitHub CLI (gh) is required to publish release notes and assets" >&2
  exit 1
fi

if [ ! -f "$NOTES_FILE" ]; then
  echo "::error::Release notes file not found: $NOTES_FILE" >&2
  exit 1
fi

if ! grep -q '\.rpm' "$NOTES_FILE"; then
  echo "::error::Release notes missing RPM install section" >&2
  exit 1
fi

TITLE="Release v${VERSION}"
if gh release view "$TAG" >/dev/null 2>&1; then
  echo "Updating existing release ${TAG}"
  gh release edit "$TAG" --title "$TITLE" --notes-file "$NOTES_FILE"
else
  echo "Creating release ${TAG}"
  gh release create "$TAG" --title "$TITLE" --notes-file "$NOTES_FILE"
fi

shopt -s nullglob globstar
assets=()
while IFS= read -r -d '' f; do
  assets+=("$f")
done < <(find "$PACKAGES_DIR" \( -name '*.deb' -o -name '*.rpm' -o -name '*.tar.gz' \) -type f -print0 | sort -z)

checksum="${PACKAGES_DIR}/SHA256SUMS.txt"
[ -f "$checksum" ] && assets+=("$checksum")

if [ "${#assets[@]}" -eq 0 ]; then
  echo "::error::No release assets to upload under ${PACKAGES_DIR}" >&2
  exit 1
fi

echo "Uploading ${#assets[@]} asset(s) to ${TAG}"
gh release upload "$TAG" "${assets[@]}" --clobber

echo "=== Published release ${TAG} ==="
gh release view "$TAG" --json name,tagName,url --jq '"\(.name) \(.tagName) \(.url)"'
