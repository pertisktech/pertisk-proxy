#!/usr/bin/env bash
# Build signed release artifacts for HCP Terraform private registry.
# Output: terraform/dist/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-0.1.0}"
BIN="terraform-provider-pertisk-proxy"
DIST="$ROOT/dist"
GPG_KEY_ID="${GPG_KEY_ID:-}"

rm -rf "$DIST"
mkdir -p "$DIST"

platforms=(
  "linux/amd64"
  "linux/arm64"
  "darwin/amd64"
  "darwin/arm64"
)

echo "building $BIN $VERSION"
for p in "${platforms[@]}"; do
  os="${p%/*}"
  arch="${p#*/}"
  outdir="$DIST/${os}_${arch}"
  mkdir -p "$outdir"
  (
    cd "$ROOT"
    GOOS="$os" GOARCH="$arch" CGO_ENABLED=0 \
      go build -trimpath -ldflags="-s -w -X main.version=${VERSION}" \
      -o "$outdir/$BIN" .
  )
  (
    cd "$outdir"
    zip -q "$DIST/${BIN}_${VERSION}_${os}_${arch}.zip" "$BIN"
  )
  rm -rf "$outdir"
  echo "  + ${BIN}_${VERSION}_${os}_${arch}.zip"
done

(
  cd "$DIST"
  shasum -a 256 ${BIN}_${VERSION}_*.zip > "${BIN}_${VERSION}_SHA256SUMS"
)

if [[ -z "$GPG_KEY_ID" ]]; then
  # Prefer the pertisktech release key (often created without a passphrase).
  GPG_KEY_ID="$(
    gpg --list-secret-keys --with-colons 2>/dev/null | awk -F: '
      /^sec:/ { kid=$5; next }
      /^uid:/ && tolower($10) ~ /pertisktech|devops@pertisk\.com/ { print kid; exit }
    '
  )"
fi
if [[ -z "$GPG_KEY_ID" ]]; then
  GPG_KEY_ID="$(gpg --list-secret-keys --with-colons 2>/dev/null | awk -F: '/^sec:/{print $5; exit}')"
fi

if [[ -z "$GPG_KEY_ID" ]]; then
  echo "no GPG secret key found; generate one then re-run:" >&2
  echo "  gpg --batch --passphrase '' --quick-generate-key 'pertisktech <devops@pertisk.com>' default default never" >&2
  exit 1
fi

echo "signing with GPG key $GPG_KEY_ID"
sums="$DIST/${BIN}_${VERSION}_SHA256SUMS"
sign_ok=0
# Unprotected keys / CI: empty or GPG_PASSPHRASE via loopback.
if gpg --batch --yes --pinentry-mode loopback \
  --passphrase "${GPG_PASSPHRASE-}" \
  --detach-sign -u "$GPG_KEY_ID" "$sums" 2>/dev/null; then
  sign_ok=1
fi
# Interactive pinentry for passphrase-protected keys.
if [[ "$sign_ok" -ne 1 ]]; then
  echo "loopback signing failed; trying interactive pinentry…"
  gpg --yes --detach-sign -u "$GPG_KEY_ID" "$sums"
fi

gpg --armor --export "$GPG_KEY_ID" > "$DIST/gpg-public.asc"
echo "$GPG_KEY_ID" > "$DIST/gpg-key-id.txt"

echo "artifacts in $DIST"
ls -la "$DIST"
