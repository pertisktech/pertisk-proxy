#!/usr/bin/env bash
# Mirror this terraform/ tree into the public Registry repo:
#   https://github.com/pertisktech/terraform-provider-pertisk-proxy
#
# Usage (from terraform/):
#   make sync-provider-repo
#   PROVIDER_REPO=https://github.com/pertisktech/terraform-provider-pertisk-proxy.git ./scripts/sync-provider-repo.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROVIDER_REPO="${PROVIDER_REPO:-https://github.com/pertisktech/terraform-provider-pertisk-proxy.git}"
BRANCH="${PROVIDER_BRANCH:-main}"
WORKDIR="${TMPDIR:-/tmp}/terraform-provider-pertisk-proxy-sync-$$"

cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

echo "syncing $ROOT → $PROVIDER_REPO ($BRANCH)"
git clone --depth 1 --branch "$BRANCH" "$PROVIDER_REPO" "$WORKDIR"

# Replace tracked content (keep .git).
find "$WORKDIR" -mindepth 1 -maxdepth 1 ! -name '.git' -exec rm -rf {} +

rsync -a \
  --exclude '.git/' \
  --exclude 'bin/' \
  --exclude 'dist/' \
  --exclude '.terraform/' \
  --exclude '*.tfstate*' \
  --exclude '.DS_Store' \
  "$ROOT"/ "$WORKDIR"/

# Public provider repo does not need the HCP private publish helper.
rm -f "$WORKDIR/scripts/publish-hcp.sh"

cd "$WORKDIR"
git add -A
if git diff --cached --quiet; then
  echo "already up to date — nothing to commit"
  exit 0
fi

msg="sync from pertisk-proxy/terraform @ $(git -C "$ROOT/.." rev-parse --short HEAD 2>/dev/null || echo unknown)"
git -c user.name="${GIT_AUTHOR_NAME:-pertisktech}" \
    -c user.email="${GIT_AUTHOR_EMAIL:-devops@pertisk.com}" \
    commit -m "$msg"

git push origin "HEAD:$BRANCH"
echo "pushed → $PROVIDER_REPO ($BRANCH)"
echo "next: cd terraform && make release && create GitHub Release v\$VERSION with dist/"
