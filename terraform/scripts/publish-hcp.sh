#!/usr/bin/env bash
# Publish dist/ artifacts to HCP Terraform private registry (org pertisktech).
# Requires: terraform login (app.terraform.io) or TF_TOKEN_app_terraform_io
# Docs: https://developer.hashicorp.com/terraform/cloud-docs/registry/publish-providers
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"
VERSION="${VERSION:-0.1.0}"
ORG="${ORG:-pertisktech}"
NAME="${NAME:-pertisk-proxy}"
BIN="terraform-provider-pertisk-proxy"
HOST="${TF_HOST:-app.terraform.io}"

if [[ ! -d "$DIST" ]]; then
  echo "missing $DIST — run: make release" >&2
  exit 1
fi

token="${TF_TOKEN_app_terraform_io:-}"
if [[ -z "$token" && -f "${HOME}/.terraform.d/credentials.tfrc.json" ]]; then
  token="$(python3 - <<'PY'
import json, os
path = os.path.expanduser("~/.terraform.d/credentials.tfrc.json")
print(json.load(open(path))["credentials"]["app.terraform.io"]["token"])
PY
)"
fi
if [[ -z "$token" ]]; then
  echo "no HCP Terraform token; run: terraform login" >&2
  exit 1
fi

api() {
  local method="$1" path="$2"
  shift 2
  curl -sS -X "$method" \
    -H "Authorization: Bearer ${token}" \
    -H "Content-Type: application/vnd.api+json" \
    "https://${HOST}${path}" \
    "$@"
}

json_get() {
  python3 -c 'import json,sys; d=json.load(sys.stdin)
def dig(o, path):
  for p in path.split("."):
    if isinstance(o, dict): o=o.get(p)
    else: return None
  return o
print(dig(d, sys.argv[1]) or "")' "$1"
}

KEY_ID="$(tr -d '[:space:]' < "$DIST/gpg-key-id.txt")"
ASC="$(cat "$DIST/gpg-public.asc")"

echo "ensuring GPG key $KEY_ID in org $ORG"
gpg_list="$(api GET "/api/registry/private/v2/gpg-keys?filter%5Bnamespace%5D=${ORG}")"
existing="$(printf '%s' "$gpg_list" | python3 -c 'import json,sys
d=json.load(sys.stdin); kid=sys.argv[1]
for x in d.get("data") or []:
  if x.get("attributes",{}).get("key-id")==kid: print("yes"); break
' "$KEY_ID")"
if [[ "$existing" != "yes" ]]; then
  api POST "/api/registry/private/v2/gpg-keys" --data @- >/dev/null <<EOF
{"data":{"type":"gpg-keys","attributes":{"namespace":"${ORG}","ascii-armor":$(python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))' <<<"$ASC")}}}
EOF
  echo "  uploaded GPG public key"
else
  echo "  GPG key already present"
fi

echo "ensuring provider ${ORG}/${NAME}"
prov="$(api GET "/api/v2/organizations/${ORG}/registry-providers/private/${ORG}/${NAME}" || true)"
if ! printf '%s' "$prov" | grep -q '"type":"registry-providers"'; then
  api POST "/api/v2/organizations/${ORG}/registry-providers" --data @- >/dev/null <<EOF
{"data":{"type":"registry-providers","attributes":{"name":"${NAME}","namespace":"${ORG}","registry-name":"private"}}}
EOF
  echo "  created provider"
else
  echo "  provider already exists"
fi

echo "creating version ${VERSION}"
ver_resp="$(api POST "/api/v2/organizations/${ORG}/registry-providers/private/${ORG}/${NAME}/versions" --data @- <<EOF
{"data":{"type":"registry-provider-versions","attributes":{"version":"${VERSION}","key-id":"${KEY_ID}","protocols":["5.0","6.0"]}}}
EOF
)"
shasums_upload="$(printf '%s' "$ver_resp" | json_get 'data.links.shasums-upload')"
shasums_sig_upload="$(printf '%s' "$ver_resp" | json_get 'data.links.shasums-sig-upload')"
if [[ -z "$shasums_upload" || -z "$shasums_sig_upload" ]]; then
  echo "create version failed:" >&2
  printf '%s\n' "$ver_resp" >&2
  exit 1
fi

echo "uploading SHA256SUMS"
curl -sS -X PUT --upload-file "$DIST/${BIN}_${VERSION}_SHA256SUMS" "$shasums_upload" >/dev/null
curl -sS -X PUT --upload-file "$DIST/${BIN}_${VERSION}_SHA256SUMS.sig" "$shasums_sig_upload" >/dev/null

platforms=(
  "linux:amd64"
  "linux:arm64"
  "darwin:amd64"
  "darwin:arm64"
)

for p in "${platforms[@]}"; do
  os="${p%:*}"; arch="${p#*:}"
  filename="${BIN}_${VERSION}_${os}_${arch}.zip"
  file="$DIST/$filename"
  shasum="$(awk -v f="$filename" '$2==f {print $1}' "$DIST/${BIN}_${VERSION}_SHA256SUMS")"
  echo "platform ${os}_${arch}"
  plat_resp="$(api POST "/api/v2/organizations/${ORG}/registry-providers/private/${ORG}/${NAME}/versions/${VERSION}/platforms" --data @- <<EOF
{"data":{"type":"registry-provider-platforms","attributes":{"os":"${os}","arch":"${arch}","shasum":"${shasum}","filename":"${filename}"}}}
EOF
)"
  upload="$(printf '%s' "$plat_resp" | json_get 'data.links.provider-binary-upload')"
  if [[ -z "$upload" ]]; then
    echo "create platform failed:" >&2
    printf '%s\n' "$plat_resp" >&2
    exit 1
  fi
  curl -sS -X PUT --upload-file "$file" "$upload" >/dev/null
done

echo
echo "published → https://${HOST}/app/${ORG}/registry/providers/private/${ORG}/${NAME}/${VERSION}"
echo "use in Terraform:"
echo "  source = \"${HOST}/${ORG}/${NAME}\""
echo "  version = \"${VERSION}\""
