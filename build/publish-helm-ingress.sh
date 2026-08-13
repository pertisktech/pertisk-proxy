#!/usr/bin/env bash
# Package pertisk-ingress Helm chart (and optionally upload to the Pertisk chart repository).
#
# Usage:
#   VERSION=0.1.0 ./build/publish-helm-ingress.sh                 # package + upload
#   VERSION=0.1.0 PACKAGE_ONLY=1 ./build/publish-helm-ingress.sh  # package to release/ only
#   HELM_CHART_TOKEN=<jwt> VERSION=0.1.0 ./build/publish-helm-ingress.sh
#
# Auth (required unless PACKAGE_ONLY=1; one of):
#   HELM_CHART_TOKEN — Bearer JWT from chart repo login
#   HELM_USER + HELM_PASSWORD — login via /api/auth/login (username + password)
#
# Env:
#   HELM_CHART_REPO_URL — default https://chart.cloud.pertisksoft.net
#   HELM_CHART_DIR      — default deploy/helm/pertisk-ingress
#   RELEASE_DIR         — default release (local .tgz output)
#   PACKAGE_ONLY        — 1 = skip upload (write .tgz to RELEASE_DIR)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

VERSION="${VERSION:-}"
HELM_CHART_REPO_URL="${HELM_CHART_REPO_URL:-https://chart.cloud.pertisksoft.net}"
HELM_CHART_DIR="${HELM_CHART_DIR:-deploy/helm/pertisk-ingress}"
RELEASE_DIR="${RELEASE_DIR:-release}"
PACKAGE_ONLY="${PACKAGE_ONLY:-0}"

if [ -z "$VERSION" ]; then
  echo "VERSION is required (e.g. 0.1.0)" >&2
  exit 1
fi

if ! command -v helm >/dev/null 2>&1; then
  echo "helm CLI not found" >&2
  exit 1
fi

if [ "$PACKAGE_ONLY" != "1" ] && ! command -v curl >/dev/null 2>&1; then
  echo "curl not found" >&2
  exit 1
fi

auth_token() {
  if [ -n "${HELM_CHART_TOKEN:-}" ]; then
    printf '%s' "$HELM_CHART_TOKEN"
    return 0
  fi
  if [ -z "${HELM_USER:-}" ] || [ -z "${HELM_PASSWORD:-}" ]; then
    echo "Set HELM_CHART_TOKEN or HELM_USER + HELM_PASSWORD" >&2
    return 1
  fi
  local resp token
  resp="$(curl -fsS -X POST "${HELM_CHART_REPO_URL%/}/api/auth/login" \
    -H "Content-Type: application/json" \
    -d "{\"username\":\"${HELM_USER}\",\"password\":\"${HELM_PASSWORD}\"}")"
  token="$(printf '%s' "$resp" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("token",""))')"
  if [ -z "$token" ]; then
    echo "Failed to obtain chart repo auth token" >&2
    printf '%s\n' "$resp" >&2
    return 1
  fi
  printf '%s' "$token"
}

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

cp -a "$REPO_ROOT/$HELM_CHART_DIR" "$WORKDIR/chart"

# Default image tag follows chart appVersion when empty (portable sed for macOS/Linux).
if sed --version >/dev/null 2>&1; then
  sed -i 's/^  tag: .*/  tag: ""/' "$WORKDIR/chart/values.yaml"
else
  sed -i '' 's/^  tag: .*/  tag: ""/' "$WORKDIR/chart/values.yaml"
fi

helm package "$WORKDIR/chart" \
  --version "$VERSION" \
  --app-version "v${VERSION}" \
  --destination "$WORKDIR"

CHART_TGZ="$(ls -1 "$WORKDIR"/pertisk-ingress-*.tgz 2>/dev/null | head -n1 || true)"
if [ -z "$CHART_TGZ" ] || [ ! -f "$CHART_TGZ" ]; then
  echo "helm package did not produce a .tgz in $WORKDIR" >&2
  ls -la "$WORKDIR" >&2 || true
  exit 1
fi

mkdir -p "$REPO_ROOT/$RELEASE_DIR"
cp -f "$CHART_TGZ" "$REPO_ROOT/$RELEASE_DIR/"
LOCAL_TGZ="$REPO_ROOT/$RELEASE_DIR/$(basename "$CHART_TGZ")"
echo "Packaged: $LOCAL_TGZ (chart=${VERSION}, appVersion=v${VERSION})"

if [ "$PACKAGE_ONLY" = "1" ]; then
  echo "PACKAGE_ONLY=1 — skipped chart repo upload."
  exit 0
fi

TOKEN="$(auth_token)"
echo "Uploading $(basename "$CHART_TGZ") to ${HELM_CHART_REPO_URL} (appVersion=v${VERSION})..."

HTTP_CODE="$(curl -sS -o "$WORKDIR/upload.json" -w '%{http_code}' \
  -X POST "${HELM_CHART_REPO_URL%/}/api/charts" \
  -H "Authorization: Bearer ${TOKEN}" \
  -F "chart=@${CHART_TGZ}")"

if [ "$HTTP_CODE" -ge 200 ] && [ "$HTTP_CODE" -lt 300 ]; then
  cat "$WORKDIR/upload.json"
  echo ""
  echo "Published pertisk-ingress ${VERSION} (image: harbor.tools.thaidevops.co/pertisksoft/pertisk-proxy/ingress:v${VERSION})"
  echo "Install: helm repo add pertisk ${HELM_CHART_REPO_URL%/} && helm upgrade --install pertisk-proxy-ingress pertisk/pertisk-ingress --version ${VERSION} -n pertisk-proxy --create-namespace"
  exit 0
fi

echo "Upload failed (HTTP ${HTTP_CODE}):" >&2
cat "$WORKDIR/upload.json" >&2 || true
exit 1
