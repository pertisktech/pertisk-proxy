#!/usr/bin/env bash
# Write GitHub Release markdown from downloaded package artifacts.
set -euo pipefail

: "${VERSION:?VERSION required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY required}"
PACKAGES_DIR="${PACKAGES_DIR:-packages}"
BASE="https://github.com/${GITHUB_REPOSITORY}/releases/download/v${VERSION}"

deb_amd64=""
deb_arm64=""
rpm_x86_64=""
rpm_aarch64=""
tar_amd64=""
tar_arm64=""

while IFS= read -r -d '' f; do
  name=$(basename "$f")
  case "$name" in
    *_amd64.deb) deb_amd64="$name" ;;
    *_arm64.deb) deb_arm64="$name" ;;
    *.x86_64.rpm) rpm_x86_64="$name" ;;
    *.aarch64.rpm) rpm_aarch64="$name" ;;
    *-linux-amd64.tar.gz) tar_amd64="$name" ;;
    *-linux-arm64.tar.gz) tar_arm64="$name" ;;
  esac
done < <(find "$PACKAGES_DIR" \( -name '*.deb' -o -name '*.rpm' -o -name '*.tar.gz' \) -type f -print0 | sort -z)

cat <<EOF
## Packages

| Format | Architecture | File |
|--------|--------------|------|
EOF

for f in $(find "$PACKAGES_DIR" -name '*.deb' -type f | sort); do
  n=$(basename "$f")
  arch="amd64"
  [[ "$n" == *_arm64.deb ]] && arch="arm64"
  echo "| DEB | ${arch} | \`${n}\` |"
done

for f in $(find "$PACKAGES_DIR" -name '*.rpm' -type f | sort); do
  n=$(basename "$f")
  arch="x86_64"
  [[ "$n" == *.aarch64.rpm ]] && arch="aarch64"
  echo "| RPM | ${arch} | \`${n}\` |"
done

for f in $(find "$PACKAGES_DIR" -name '*.tar.gz' -type f | sort); do
  n=$(basename "$f")
  arch="amd64"
  [[ "$n" == *-arm64.tar.gz ]] && arch="arm64"
  echo "| Tarball | ${arch} | \`${n}\` |"
done

cat <<'EOF'

## Installation

EOF

if [ -n "$deb_amd64" ]; then
  cat <<EOF
### Debian/Ubuntu (x86_64 / amd64)
\`\`\`bash
wget ${BASE}/${deb_amd64}
sudo dpkg -i ${deb_amd64}
\`\`\`

EOF
fi

if [ -n "$deb_arm64" ]; then
  cat <<EOF
### Debian/Ubuntu (arm64)
\`\`\`bash
wget ${BASE}/${deb_arm64}
sudo dpkg -i ${deb_arm64}
\`\`\`

EOF
fi

if [ -n "$rpm_x86_64" ]; then
  cat <<EOF
### RHEL/CentOS/Fedora/Rocky/Alma (x86_64)
\`\`\`bash
wget ${BASE}/${rpm_x86_64}
sudo rpm -i ${rpm_x86_64}
\`\`\`

EOF
fi

if [ -n "$rpm_aarch64" ]; then
  cat <<EOF
### RHEL/CentOS/Fedora/Rocky/Alma (aarch64)
\`\`\`bash
wget ${BASE}/${rpm_aarch64}
sudo rpm -i ${rpm_aarch64}
\`\`\`

EOF
fi

if [ -n "$tar_amd64" ]; then
  cat <<EOF
### Binary tarball (amd64)
\`\`\`bash
wget ${BASE}/${tar_amd64}
tar -xzf ${tar_amd64}
PERTISK_DB_PATH=./data/proxy.sqlite ./usr/bin/pertisk-proxy
\`\`\`

EOF
fi

if [ -n "$tar_arm64" ]; then
  cat <<EOF
### Binary tarball (arm64)
\`\`\`bash
wget ${BASE}/${tar_arm64}
tar -xzf ${tar_arm64}
PERTISK_DB_PATH=./data/proxy.sqlite ./usr/bin/pertisk-proxy
\`\`\`

EOF
fi

cat <<EOF
## Post Installation
\`\`\`bash
# Verify capabilities (port binding)
getcap /usr/bin/pertisk-proxy
# Should show: cap_net_bind_service=ep

sudo systemctl start pertisk-proxy
sudo systemctl enable pertisk-proxy
sudo systemctl status pertisk-proxy
\`\`\`

### Running as root (optional)
Two systemd units are provided:
- \`pertisk-proxy.service\` (default, runs as user \`pertisk-proxy\` with cap_net_bind_service)
- \`pertisk-proxy-root.service\` (runs as root)

\`\`\`bash
sudo systemctl disable --now pertisk-proxy || true
sudo systemctl enable --now pertisk-proxy-root
\`\`\`

## HTTP/3 (QUIC)
UDP buffer settings are applied from \`/etc/sysctl.d/99-pertisk-proxy.conf\`. Reload with \`sudo sysctl -p /etc/sysctl.d/99-pertisk-proxy.conf\` if needed.

## Docker (Harbor)

### Proxy (standalone)
\`\`\`bash
docker pull ${HARBOR_PROXY_IMAGE:-ghcr.io/pertisktech/pertisk-proxy/proxy}:v${VERSION}
\`\`\`

### Ingress (Kubernetes controller)
\`\`\`bash
docker pull ${HARBOR_INGRESS_IMAGE:-ghcr.io/pertisktech/pertisk-proxy/ingress}:v${VERSION}
\`\`\`

### Helm (pertisk-ingress)
\`\`\`bash
helm repo add pertisk ${HELM_CHART_REPO_URL:-https://charts.example.com}
helm repo update
helm upgrade --install pertisk-proxy-ingress pertisk/pertisk-ingress \\
  --version ${VERSION} -n pertisk-proxy --create-namespace
\`\`\`

## Verify checksums
\`\`\`bash
sha256sum -c SHA256SUMS.txt
\`\`\`
EOF
