#!/usr/bin/env bash
# Build signed release artifacts for Terraform Registry (public) and HCP private registry.
# Output: terraform/dist/
# Public registry also requires a GitHub repo named terraform-provider-pertisk-proxy.
# See: https://developer.hashicorp.com/terraform/registry/providers/publishing
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-0.1.0}"
NAME="pertisk-proxy"
BIN="terraform-provider-${NAME}"
# Public registry expects the binary inside the zip to include _vVERSION.
BIN_RELEASE="${BIN}_v${VERSION}"
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

echo "building $BIN_RELEASE"
for p in "${platforms[@]}"; do
  os="${p%/*}"
  arch="${p#*/}"
  outdir="$DIST/${os}_${arch}"
  mkdir -p "$outdir"
  (
    cd "$ROOT"
    GOOS="$os" GOARCH="$arch" CGO_ENABLED=0 \
      go build -trimpath -ldflags="-s -w -X main.version=${VERSION}" \
      -o "$outdir/$BIN_RELEASE" .
  )
  (
    cd "$outdir"
    zip -q "$DIST/${BIN}_${VERSION}_${os}_${arch}.zip" "$BIN_RELEASE"
  )
  rm -rf "$outdir"
  echo "  + ${BIN}_${VERSION}_${os}_${arch}.zip"
done

# Required by the public Terraform Registry.
cat > "$DIST/${BIN}_${VERSION}_manifest.json" <<EOF
{
  "version": 1,
  "metadata": {
    "protocol_versions": ["5.0", "6.0"]
  }
}
EOF
cp "$DIST/${BIN}_${VERSION}_manifest.json" "$ROOT/terraform-registry-manifest.json"

(
  cd "$DIST"
  shasum -a 256 ${BIN}_${VERSION}_*.zip ${BIN}_${VERSION}_manifest.json > "${BIN}_${VERSION}_SHA256SUMS"
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
if gpg --batch --yes --pinentry-mode loopback \
  --passphrase "${GPG_PASSPHRASE-}" \
  --detach-sign -u "$GPG_KEY_ID" "$sums" 2>/dev/null; then
  sign_ok=1
fi
if [[ "$sign_ok" -ne 1 ]]; then
  echo "loopback signing failed; trying interactive pinentry…"
  gpg --yes --detach-sign -u "$GPG_KEY_ID" "$sums"
fi

gpg --armor --export "$GPG_KEY_ID" > "$DIST/gpg-public.asc"
echo "$GPG_KEY_ID" > "$DIST/gpg-key-id.txt"

echo "artifacts in $DIST"
ls -la "$DIST"
echo
echo "Public registry next steps:"
echo "  1. Create public GitHub repo: https://github.com/pertisktech/terraform-provider-pertisk-proxy"
echo "  2. Push this provider code there (repo name MUST match terraform-provider-pertisk-proxy)"
echo "  3. Upload GPG public key at https://registry.terraform.io/ → User Settings → Signing Keys"
echo "  4. GitHub Release tag v${VERSION} with all files from dist/"
echo "  5. Publish → Provider at https://registry.terraform.io/"
echo "Docs: https://developer.hashicorp.com/terraform/registry/providers/publishing"
