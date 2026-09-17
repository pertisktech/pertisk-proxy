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
  # Prefer passphrase-free public-registry key, then legacy pertisktech key.
  GPG_KEY_ID="$(
    gpg --list-secret-keys --with-colons 2>/dev/null | awk -F: '
      /^sec:/ { kid=$5; next }
      /^uid:/ && tolower($10) ~ /pertisktech-registry/ { print kid; exit }
    '
  )"
fi
if [[ -z "$GPG_KEY_ID" ]]; then
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
rm -f "${sums}.sig"
sign_ok=0

# Prefer GnuPG 1.4 in Docker — classic OpenPGP sigs (no issuer-fpr/manu
# subpackets). Registry rejects many GnuPG 2.4/2.5 signatures as "Invalid signature".
if command -v docker >/dev/null 2>&1; then
  sign_dir="$(mktemp -d "${TMPDIR:-/tmp}/gpg-registry-sign.XXXXXX")"
  cleanup_sign() { rm -rf "$sign_dir"; }
  trap cleanup_sign EXIT
  cp "$sums" "$sign_dir/SHA256SUMS"
  if [[ -n "${GPG_PASSPHRASE-}" ]]; then
    printf '%s' "$GPG_PASSPHRASE" | gpg --batch --pinentry-mode loopback --passphrase-fd 0 \
      --export-secret-keys --armor "$GPG_KEY_ID" > "$sign_dir/secret.asc"
  else
    gpg --batch --pinentry-mode loopback --passphrase '' \
      --export-secret-keys --armor "$GPG_KEY_ID" > "$sign_dir/secret.asc" 2>/dev/null \
      || gpg --export-secret-keys --armor "$GPG_KEY_ID" > "$sign_dir/secret.asc"
  fi
  if docker run --rm \
      -e PASS="${GPG_PASSPHRASE-}" \
      -e KEY_ID="$GPG_KEY_ID" \
      -v "$sign_dir:/work" -w /work \
      debian:bookworm-slim bash -lc '
        set -euo pipefail
        apt-get update -qq
        DEBIAN_FRONTEND=noninteractive apt-get install -y -qq gnupg1 >/dev/null
        export GNUPGHOME=/tmp/gnupg-home
        mkdir -p "$GNUPGHOME" && chmod 700 "$GNUPGHOME"
        if [[ -n "${PASS:-}" ]]; then
          printf "%s" "$PASS" | gpg1 --batch --passphrase-fd 0 --import secret.asc
          printf "%s" "$PASS" | gpg1 --batch --yes --passphrase-fd 0 \
            --digest-algo SHA512 --detach-sign -u "$KEY_ID" SHA256SUMS
        else
          gpg1 --batch --import secret.asc
          gpg1 --batch --yes --digest-algo SHA512 --detach-sign -u "$KEY_ID" SHA256SUMS
        fi
        gpg1 --verify SHA256SUMS.sig SHA256SUMS
      '; then
    cp "$sign_dir/SHA256SUMS.sig" "${sums}.sig"
    sign_ok=1
    echo "  signed with GnuPG 1.4 (docker/debian) — Registry-compatible"
  fi
  trap - EXIT
  cleanup_sign
fi

if [[ "$sign_ok" -ne 1 ]]; then
  echo "docker signing unavailable; falling back to local gpg (may fail Registry verification on GnuPG 2.5)"
  if [[ -n "${GPG_PASSPHRASE-}" ]]; then
    if printf '%s' "$GPG_PASSPHRASE" | gpg --batch --yes --pinentry-mode loopback \
      --passphrase-fd 0 \
      --compatibility-flags no-manu \
      --detach-sign -u "$GPG_KEY_ID" "$sums"; then
      sign_ok=1
    fi
  else
    if gpg --batch --yes --pinentry-mode loopback \
      --passphrase '' \
      --compatibility-flags no-manu \
      --detach-sign -u "$GPG_KEY_ID" "$sums" 2>/dev/null; then
      sign_ok=1
    fi
  fi
fi
if [[ "$sign_ok" -ne 1 ]]; then
  echo "loopback signing failed; trying interactive pinentry…"
  gpg --yes --compatibility-flags no-manu --detach-sign -u "$GPG_KEY_ID" "$sums"
fi

gpg --armor --export "$GPG_KEY_ID" > "$DIST/gpg-public.asc"
echo "$GPG_KEY_ID" > "$DIST/gpg-key-id.txt"

echo "artifacts in $DIST"
ls -la "$DIST"
echo
echo "Public registry next steps:"
echo "  1. Signing Keys (org pertisktech): paste $DIST/gpg-public.asc"
echo "     fingerprint: $(gpg --with-colons --fingerprint \"$GPG_KEY_ID\" 2>/dev/null | awk -F: '/^fpr:/{print $10; exit}')"
echo "  2. Resync provider at https://registry.terraform.io/providers/pertisktech/pertisk-proxy"
echo "  3. Confirm: curl -s https://registry.terraform.io/v1/providers/pertisktech/pertisk-proxy/versions"
echo "Docs: https://developer.hashicorp.com/terraform/registry/providers/publishing"
