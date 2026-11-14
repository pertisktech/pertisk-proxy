#!/bin/sh
# pertisk-proxy Linux installer (proxy mode).
#
#   curl -sfL https://get.proxy.pertisk.com | sh -
#   curl -sfL https://github.com/pertisktech/pertisk-proxy/releases/latest/download/get.sh | sh -
#
# Optional env:
#   INSTALL_PERTISK_VERSION=0.1.87     pin a release (with or without leading v)
#   INSTALL_PERTISK_TYPE=deb|rpm|tar   skip distro detection
#   INSTALL_PERTISK_DB=sqlite|scylla   admin config DB (default sqlite; scylla installs native ScyllaDB)
#   INSTALL_PERTISK_SKIP_START=1       install only, do not start systemd
#   INSTALL_PERTISK_SKIP_ENABLE=1      start now but do not enable on boot
#   INSTALL_PERTISK_REPO=org/repo      default pertisktech/pertisk-proxy
#   INSTALL_PERTISK_GITHUB_TOKEN=...   GitHub token (private repo / rate limit)
#   INSTALL_PERTISK_DRY_RUN=1          print actions, do not install
set -eu

REPO="${INSTALL_PERTISK_REPO:-pertisktech/pertisk-proxy}"
GITHUB_BASE="${INSTALL_PERTISK_GITHUB:-https://github.com/${REPO}}"
BIN="pertisk-proxy"
UNIT="${BIN}.service"
# Pinned ScyllaDB open-source series for apt/yum repos.
SCYLLA_SERIES="${INSTALL_PERTISK_SCYLLA_SERIES:-6.2}"

info() { printf '[INFO]  %s\n' "$*"; }
warn() { printf '[WARN]  %s\n' "$*" >&2; }
fatal() { printf '[ERROR] %s\n' "$*" >&2; exit 1; }

ADMIN_DB="$(printf '%s' "${INSTALL_PERTISK_DB:-sqlite}" | tr '[:upper:]' '[:lower:]')"
case "$ADMIN_DB" in
  ""|sqlite|rusqlite) ADMIN_DB="sqlite" ;;
  scylla|scylladb|cassandra) ADMIN_DB="scylla" ;;
  *) fatal "INSTALL_PERTISK_DB must be sqlite or scylla (got ${INSTALL_PERTISK_DB})" ;;
esac

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fatal "missing required command: $1"
}

if [ "$(uname -s)" != "Linux" ]; then
  fatal "this installer supports Linux only (got $(uname -s))"
fi

need_cmd curl
need_cmd uname

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
else
  command -v sudo >/dev/null 2>&1 || fatal "run as root, or install sudo"
  SUDO="sudo"
  info "not root; privileged steps will use sudo"
fi

run() {
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" = "1" ]; then
    printf '[DRY]   %s\n' "$*"
    return 0
  fi
  if [ -n "$SUDO" ]; then
    # shellcheck disable=SC2086
    $SUDO "$@"
  else
    "$@"
  fi
}

github_curl() {
  _tok="${INSTALL_PERTISK_GITHUB_TOKEN:-${GITHUB_TOKEN:-}}"
  if [ -n "${_tok}" ]; then
    curl -H "Authorization: Bearer ${_tok}" "$@"
  else
    curl "$@"
  fi
}

# Detect amd64|arm64 (deb/tarball) and rpm_arch.
detect_arch() {
  _m="$(uname -m)"
  case "$_m" in
    x86_64|amd64)
      GOARCH=amd64
      RPM_ARCH=x86_64
      ;;
    aarch64|arm64)
      GOARCH=arm64
      RPM_ARCH=aarch64
      ;;
    *)
      fatal "unsupported architecture: ${_m} (need x86_64 or aarch64)"
      ;;
  esac
}

detect_pkg_type() {
  if [ -n "${INSTALL_PERTISK_TYPE:-}" ]; then
    PKG_TYPE="${INSTALL_PERTISK_TYPE}"
    case "$PKG_TYPE" in
      deb|rpm|tar) ;;
      *) fatal "INSTALL_PERTISK_TYPE must be deb, rpm, or tar" ;;
    esac
    return
  fi
  _id=""
  _like=""
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    _id="${ID:-}"
    _like="${ID_LIKE:-}"
  fi
  _all="${_id} ${_like}"
  case " ${_all} " in
    *" debian "*|*" ubuntu "*|*" linuxmint "*|*" pop "*|*" raspios "*)
      PKG_TYPE=deb
      ;;
    *" rhel "*|*" fedora "*|*" centos "*|*" rocky "*|*" alma "*|*" amzn "*|*" suse "*|*" sles "*|*" opensuse "*)
      PKG_TYPE=rpm
      ;;
    *)
      if [ -f /etc/debian_version ]; then
        PKG_TYPE=deb
      elif command -v rpm >/dev/null 2>&1 && { command -v dnf >/dev/null 2>&1 || command -v yum >/dev/null 2>&1 || command -v zypper >/dev/null 2>&1; }; then
        PKG_TYPE=rpm
      else
        PKG_TYPE=tar
        warn "no native package manager detected; using tarball"
      fi
      ;;
  esac
}

resolve_tag() {
  if [ -n "${INSTALL_PERTISK_VERSION:-}" ]; then
    TAG="${INSTALL_PERTISK_VERSION}"
    TAG="${TAG#v}"
    TAG="v${TAG}"
    info "using pinned version ${TAG}"
    return
  fi
  _latest="${GITHUB_BASE}/releases/latest"
  info "resolving latest release from ${_latest}"
  _url="$(github_curl -fsSL --retry 3 --retry-delay 1 -o /dev/null -w '%{url_effective}' "${_latest}")" \
    || fatal "failed to resolve latest GitHub release (check network / GitHub)"
  _url="${_url%/}"
  TAG="${_url##*/}"
  case "$TAG" in
    v*.*.*|*.*.*) ;;
    *) fatal "could not parse release tag from ${_url}" ;;
  esac
  case "$TAG" in
    v*) ;;
    *) TAG="v${TAG}" ;;
  esac
  info "latest release is ${TAG}"
}

download() {
  _url="$1"
  _dest="$2"
  info "downloading ${_url}"
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" = "1" ]; then
    return 0
  fi
  github_curl -fL --retry 3 --retry-delay 1 --connect-timeout 15 --max-time 300 \
    -o "${_dest}" "${_url}" \
    || fatal "download failed: ${_url}"
}

verify_sha256() {
  _hash="$1"
  _file="$2"
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" = "1" ]; then
    return 0
  fi
  [ -f "$_file" ] || fatal "missing file for checksum: ${_file}"
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "${_hash}" "${_file}" | sha256sum -c -
  elif command -v shasum >/dev/null 2>&1; then
    printf '%s  %s\n' "${_hash}" "${_file}" | shasum -a 256 -c -
  else
    fatal "need sha256sum (coreutils) to verify the package"
  fi
}

# Match SHA256SUMS.txt (hash may be followed by "release/name" or "name").
pick_from_sums() {
  _sums="$1"
  _glob="$2"
  _line=""
  while IFS= read -r _line || [ -n "${_line}" ]; do
    case "${_line}" in
      ''|'#'*) continue ;;
    esac
    _hash="${_line%% *}"
    _path="${_line#* }"
    _path="${_path# }"
    _path="${_path#\*}"
    _base="${_path##*/}"
    case "${_base}" in
      *tunnel*|*ingress*) continue ;;
      ${_glob})
        printf '%s %s\n' "${_hash}" "${_base}"
        return 0
        ;;
    esac
  done < "${_sums}"
  return 1
}

install_deb() {
  _pkg="$1"
  if command -v apt-get >/dev/null 2>&1; then
    info "installing Debian package with apt"
    run apt-get update -qq || true
    if ! run apt-get install -y "./${_pkg}"; then
      run dpkg -i "${_pkg}" || true
      run apt-get install -f -y
    fi
  else
    need_cmd dpkg
    info "installing Debian package with dpkg"
    run dpkg -i "${_pkg}" || run apt-get install -f -y
  fi
}

install_rpm() {
  _pkg="$1"
  if command -v dnf >/dev/null 2>&1; then
    info "installing RPM with dnf"
    run dnf install -y "./${_pkg}"
  elif command -v yum >/dev/null 2>&1; then
    info "installing RPM with yum"
    run yum install -y "./${_pkg}"
  elif command -v zypper >/dev/null 2>&1; then
    info "installing RPM with zypper"
    run zypper --non-interactive install --allow-unsigned-rpm "./${_pkg}"
  else
    need_cmd rpm
    info "installing RPM with rpm -Uvh"
    run rpm -Uvh "${_pkg}"
  fi
}

ensure_user() {
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" = "1" ]; then
    return 0
  fi
  if ! getent group pertisk-proxy >/dev/null 2>&1; then
    run groupadd --system pertisk-proxy
  fi
  if ! getent passwd pertisk-proxy >/dev/null 2>&1; then
    run useradd --system --gid pertisk-proxy --home-dir /var/lib/pertisk-proxy \
      --shell /usr/sbin/nologin --comment "pertisk-proxy" pertisk-proxy
  fi
}

install_tar() {
  _tar="$1"
  _extract="$(mktemp -d)"
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" != "1" ]; then
    tar -xzf "${_tar}" -C "${_extract}"
    [ -x "${_extract}/usr/bin/${BIN}" ] || fatal "tarball missing usr/bin/${BIN}"
  fi
  ensure_user
  run mkdir -p /usr/bin /etc/pertisk-proxy /var/lib/pertisk-proxy/certs \
    /var/log/pertisk-proxy /usr/share/pertisk-proxy /lib/systemd/system
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" != "1" ]; then
    run cp "${_extract}/usr/bin/${BIN}" "/usr/bin/${BIN}"
    run chmod 0755 "/usr/bin/${BIN}"
    if [ -d "${_extract}/admin/dist" ]; then
      run mkdir -p /usr/share/pertisk-proxy/admin
      run cp -a "${_extract}/admin/dist" /usr/share/pertisk-proxy/admin/
    elif [ -d "${_extract}/usr/share/pertisk-proxy/admin" ]; then
      run cp -a "${_extract}/usr/share/pertisk-proxy/admin" /usr/share/pertisk-proxy/
    fi
    rm -rf "${_extract}"
  fi
  write_proxy_conf
  write_proxy_unit
  run chown -R pertisk-proxy:pertisk-proxy /var/lib/pertisk-proxy /var/log/pertisk-proxy || true
  if command -v setcap >/dev/null 2>&1; then
    run setcap 'cap_net_bind_service=+ep' "/usr/bin/${BIN}" || warn "setcap failed; systemd AmbientCapabilities still apply"
  fi
  if command -v systemctl >/dev/null 2>&1; then
    run systemctl daemon-reload
  fi
}

write_proxy_conf() {
  _conf_path="/etc/pertisk-proxy/${BIN}.conf"
  if [ -f "${_conf_path}" ] && [ "${INSTALL_PERTISK_DRY_RUN:-}" != "1" ] && [ "${ADMIN_DB}" = "sqlite" ]; then
    info "keeping existing ${_conf_path}"
    return 0
  fi
  info "writing ${_conf_path} (admin DB=${ADMIN_DB})"
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" = "1" ]; then
    return 0
  fi
  _conf="$(mktemp)"
  if [ "${ADMIN_DB}" = "scylla" ]; then
    cat > "${_conf}" <<'CONF'
# pertisk-proxy (proxy mode) — ScyllaDB admin config backend
PERTISK_DB_BACKEND=scylla
PERTISK_SCYLLA_HOSTS=127.0.0.1:9042
PERTISK_SCYLLA_KEYSPACE=pertisk_proxy
LISTEN_HTTP=[::]:80
LISTEN_HTTPS=[::]:443
LISTEN_H3_UDP=[::]:443
ENABLE_H3=true
PERTISK_MANAGEMENT_ADDR=127.0.0.1:9080
PERTISK_PROXY_MODE=auto
PERTISK_LOG_LEVEL=info
CONF
  else
    cat > "${_conf}" <<'CONF'
# pertisk-proxy (proxy mode)
PERTISK_DB_BACKEND=sqlite
PERTISK_DB_PATH=/var/lib/pertisk-proxy/proxy.sqlite
LISTEN_HTTP=[::]:80
LISTEN_HTTPS=[::]:443
LISTEN_H3_UDP=[::]:443
ENABLE_H3=true
PERTISK_MANAGEMENT_ADDR=127.0.0.1:9080
PERTISK_PROXY_MODE=auto
PERTISK_LOG_LEVEL=info
CONF
  fi
  run cp "${_conf}" "${_conf_path}"
  rm -f "${_conf}"
}

write_proxy_unit() {
  info "writing systemd unit ${UNIT}"
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" = "1" ]; then
    return 0
  fi
  _unit="$(mktemp)"
  if [ "${ADMIN_DB}" = "scylla" ]; then
    cat > "${_unit}" <<'UNIT'
[Unit]
Description=pertisk-proxy reverse proxy
After=network.target
After=network-online.target
Wants=network-online.target
After=scylla-server.service
Wants=scylla-server.service

[Service]
Type=simple
User=pertisk-proxy
Group=pertisk-proxy
WorkingDirectory=/var/lib/pertisk-proxy
EnvironmentFile=-/etc/pertisk-proxy/pertisk-proxy.conf
ExecStart=/usr/bin/pertisk-proxy
Restart=always
RestartSec=5
TimeoutStopSec=30
LimitNOFILE=1048576
TasksMax=infinity
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/pertisk-proxy /var/log/pertisk-proxy
PrivateTmp=true
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
UNIT
  else
    cat > "${_unit}" <<'UNIT'
[Unit]
Description=pertisk-proxy reverse proxy
After=network.target
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pertisk-proxy
Group=pertisk-proxy
WorkingDirectory=/var/lib/pertisk-proxy
EnvironmentFile=-/etc/pertisk-proxy/pertisk-proxy.conf
ExecStart=/usr/bin/pertisk-proxy
Restart=always
RestartSec=5
TimeoutStopSec=30
LimitNOFILE=1048576
TasksMax=infinity
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/pertisk-proxy /var/log/pertisk-proxy
PrivateTmp=true
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
UNIT
  fi
  run cp "${_unit}" /lib/systemd/system/${UNIT}
  rm -f "${_unit}"
}

# Install native ScyllaDB packages from official apt/yum repos (same-host admin DB).
install_scylladb() {
  if [ "${ADMIN_DB}" != "scylla" ]; then
    return 0
  fi
  info "INSTALL_PERTISK_DB=scylla: installing native ScyllaDB ${SCYLLA_SERIES}"

  if [ -f /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
  else
    ID=""
  fi

  case "${ID:-}" in
    debian|ubuntu)
      need_cmd apt-get
      run apt-get update -qq || true
      run apt-get install -y curl gnupg apt-transport-https ca-certificates
      run mkdir -p /etc/apt/keyrings
      if [ "${INSTALL_PERTISK_DRY_RUN:-}" != "1" ]; then
        curl -fsSL "https://downloads.scylladb.com/downloads/scylla/deb/debian/scylladb-${SCYLLA_SERIES}/dsa_key.pub" \
          | gpg --dearmor | ${SUDO} tee /etc/apt/keyrings/scylladb.gpg >/dev/null
        printf 'deb [signed-by=/etc/apt/keyrings/scylladb.gpg] https://downloads.scylladb.com/downloads/scylla/deb/debian/scylladb-%s stable main\n' \
          "${SCYLLA_SERIES}" | ${SUDO} tee /etc/apt/sources.list.d/scylla.list >/dev/null
      else
        printf '[DRY]   add Scylla apt repo scylladb-%s\n' "${SCYLLA_SERIES}"
      fi
      run apt-get update -qq
      run apt-get install -y scylla
      ;;
    rhel|centos|rocky|almalinux|fedora|amzn)
      _mgr=""
      if command -v dnf >/dev/null 2>&1; then
        _mgr=dnf
      elif command -v yum >/dev/null 2>&1; then
        _mgr=yum
      else
        fatal "need dnf or yum to install ScyllaDB on ${ID}"
      fi
      if [ "${INSTALL_PERTISK_DRY_RUN:-}" != "1" ]; then
        curl -fsSL "https://downloads.scylladb.com/rpm/centos/scylladb-${SCYLLA_SERIES}.repo" \
          | ${SUDO} tee /etc/yum.repos.d/scylla.repo >/dev/null
      else
        printf '[DRY]   add Scylla yum repo scylladb-%s\n' "${SCYLLA_SERIES}"
      fi
      run "${_mgr}" install -y scylla
      ;;
    *)
      fatal "INSTALL_PERTISK_DB=scylla: unsupported distro '${ID:-unknown}' (need Debian/Ubuntu or RHEL-like)"
      ;;
  esac

  # Developer / single-node defaults suitable for same-host admin DB.
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" != "1" ] && [ -f /etc/scylla/scylla.yaml ]; then
    if ! grep -q '^developer_mode:' /etc/scylla/scylla.yaml 2>/dev/null; then
      info "enabling Scylla developer_mode for single-node admin DB"
      printf '\ndeveloper_mode: true\n' | ${SUDO} tee -a /etc/scylla/scylla.yaml >/dev/null
    fi
  fi
  if command -v scylla_setup >/dev/null 2>&1; then
    info "running scylla_setup (non-interactive / developer-mode)"
    run scylla_setup --no-raid-setup --no-ec2-check --no-kernel-check \
      --no-verify-package --no-enable-service --no-selinux-setup \
      --no-bootparam-setup --no-ntp-setup --no-coredump-setup \
      --no-sysconfig-setup --no-io-setup --no-version-check \
      --developer-mode 1 || warn "scylla_setup returned non-zero; continuing"
  fi

  if command -v systemctl >/dev/null 2>&1; then
    run systemctl enable scylla-server || true
    run systemctl start scylla-server || warn "scylla-server failed to start; check journalctl -u scylla-server"
  fi
}

# For deb/rpm installs: ensure conf/unit reflect ADMIN_DB (packages ship SQLite defaults).
configure_admin_db_after_pkg() {
  if [ "${ADMIN_DB}" = "scylla" ]; then
    write_proxy_conf
    write_proxy_unit
    if command -v systemctl >/dev/null 2>&1; then
      run systemctl daemon-reload
    fi
  elif [ ! -f "/etc/pertisk-proxy/${BIN}.conf" ]; then
    write_proxy_conf
  fi
}

start_service() {
  if [ "${INSTALL_PERTISK_SKIP_START:-}" = "1" ]; then
    info "skipping service start (INSTALL_PERTISK_SKIP_START=1)"
    info "start later with: ${SUDO:+sudo }systemctl enable --now ${BIN}"
    return 0
  fi
  if ! command -v systemctl >/dev/null 2>&1; then
    warn "systemctl not found; start ${BIN} manually"
    return 0
  fi
  if [ "${INSTALL_PERTISK_SKIP_ENABLE:-}" = "1" ]; then
    info "starting ${BIN} (not enabled on boot)"
    run systemctl start "${BIN}"
  else
    info "enabling and starting ${BIN}"
    run systemctl enable --now "${BIN}"
  fi
  if [ "${INSTALL_PERTISK_DRY_RUN:-}" != "1" ]; then
    sleep 1
    if systemctl is-active --quiet "${BIN}"; then
      info "${BIN} is running"
    else
      warn "${BIN} did not become active; see: journalctl -u ${BIN} -e"
    fi
  fi
}

print_next_steps() {
  _ver="${TAG#v}"
  if [ "${ADMIN_DB}" = "scylla" ]; then
    _db_line="Admin DB:   ScyllaDB (PERTISK_DB_BACKEND=scylla, keyspace pertisk_proxy)"
  else
    _db_line="Admin DB:   SQLite (/var/lib/pertisk-proxy/proxy.sqlite)"
  fi
  cat <<EOF

pertisk-proxy ${_ver} is installed.

  Admin UI:   http://127.0.0.1:9080   (login admin / admin — change this)
  Config:     /etc/pertisk-proxy/${BIN}.conf
  ${_db_line}
  Data:       /var/lib/pertisk-proxy
  Status:     ${SUDO:+sudo }systemctl status ${BIN}
  Logs:       ${SUDO:+sudo }journalctl -u ${BIN} -f

Management listen defaults to 127.0.0.1:9080. Publish it behind this proxy
or SSH tunnel; do not expose :9080 on the public internet.

Kubernetes Ingress is a separate image/Helm chart, not this installer.
EOF
}

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "${WORKDIR}"; }
trap cleanup EXIT INT HUP

detect_arch
detect_pkg_type
resolve_tag

VERSION="${TAG#v}"
DOWNLOAD_BASE="${GITHUB_BASE}/releases/download/${TAG}"
SUMS_URL="${DOWNLOAD_BASE}/SHA256SUMS.txt"
SUMS="${WORKDIR}/SHA256SUMS.txt"

download "${SUMS_URL}" "${SUMS}"
if [ "${INSTALL_PERTISK_DRY_RUN:-}" != "1" ]; then
  [ -s "${SUMS}" ] || fatal "empty SHA256SUMS.txt from ${SUMS_URL}"
fi

case "$PKG_TYPE" in
  deb) _glob="${BIN}_*_${GOARCH}.deb" ;;
  rpm) _glob="${BIN}-*.${RPM_ARCH}.rpm" ;;
  tar) _glob="${BIN}-v*-linux-${GOARCH}.tar.gz" ;;
esac

info "package type ${PKG_TYPE} (${GOARCH})"
if [ "${INSTALL_PERTISK_DRY_RUN:-}" = "1" ]; then
  _picked="DRYRUN ${_glob}"
else
  _picked="$(pick_from_sums "${SUMS}" "${_glob}")" \
    || fatal "no ${PKG_TYPE} asset matching ${_glob} in ${TAG} SHA256SUMS.txt"
fi
_hash="${_picked%% *}"
_asset="${_picked#* }"
ASSET_URL="${DOWNLOAD_BASE}/${_asset}"
PKG="${WORKDIR}/${_asset}"

download "${ASSET_URL}" "${PKG}"
verify_sha256 "${_hash}" "${PKG}"

# apt/dnf want a relative path with ./
cd "${WORKDIR}"
case "$PKG_TYPE" in
  deb) install_deb "${_asset}" ;;
  rpm) install_rpm "${_asset}" ;;
  tar) install_tar "${_asset}" ;;
esac

install_scylladb
case "$PKG_TYPE" in
  deb|rpm) configure_admin_db_after_pkg ;;
esac

start_service
print_next_steps
