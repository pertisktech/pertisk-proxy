#!/bin/sh
# Debian apt mirror setup + retry helpers for flaky CDN/runners.
# Source from a Dockerfile RUN:  . /usr/local/sbin/debian-apt-setup.sh
set -eu

DEBIAN_SUITE="${DEBIAN_SUITE:-bookworm}"
PROBE_PKG="${APT_PROBE_PKG:-build-essential}"

MIRRORS="
http://deb.debian.org/debian
https://deb.debian.org/debian
http://mirrors.aliyun.com/debian
https://mirrors.aliyun.com/debian
http://mirrors.tuna.tsinghua.edu.cn/debian
https://mirrors.tuna.tsinghua.edu.cn/debian
http://mirror.csclub.uwaterloo.ca/debian
https://mirrors.kernel.org/debian
http://ftp.debian.org/debian
"

apt_update_ok() {
  if apt-get update -o Acquire::Retries=5 -o Acquire::http::Timeout=30 \
    -o Acquire::https::Timeout=30 >/tmp/apt-update.log 2>&1; then
    return 0
  fi
  cat /tmp/apt-update.log >&2 || true
  return 1
}

# apt-get update can "succeed" with empty/partial indexes — require a real package.
apt_index_usable() {
  apt-cache show "$PROBE_PKG" >/dev/null 2>&1
}

write_sources() {
  mirror="$1"
  printf '%s\n' \
    "deb ${mirror} ${DEBIAN_SUITE} main contrib" \
    "deb ${mirror} ${DEBIAN_SUITE}-updates main contrib" \
    "deb http://security.debian.org/debian-security ${DEBIAN_SUITE}-security main contrib" \
    > /etc/apt/sources.list
  rm -f /etc/apt/sources.list.d/debian.sources \
    /etc/apt/sources.list.d/*.sources \
    /etc/apt/sources.list.d/*.list 2>/dev/null || true
}

try_mirror() {
  mirror="$1"
  echo "apt: trying mirror ${mirror}" >&2
  write_sources "$mirror"
  apt_update_ok || return 1
  if ! apt_index_usable; then
    echo "apt: mirror ${mirror} updated but ${PROBE_PKG} missing — skipping" >&2
    return 1
  fi
  echo "apt: mirror ${mirror} OK (${PROBE_PKG} present)" >&2
  return 0
}

debian_apt_setup() {
  # Always rewrite sources — image defaults often update "OK" with unusable indexes
  # on restricted networks.
  for mirror in $MIRRORS; do
    if try_mirror "$mirror"; then
      return 0
    fi
    sleep 2
  done
  echo "apt: all mirrors failed for ${DEBIAN_SUITE} (need ${PROBE_PKG})" >&2
  return 1
}

apt_install_retry() {
  n=0
  max=8
  while [ "$n" -lt "$max" ]; do
    if ! apt_index_usable; then
      debian_apt_setup || true
    fi
    if DEBIAN_FRONTEND=noninteractive apt-get install -y \
      -o Acquire::Retries=5 \
      -o Acquire::http::Timeout=30 \
      -o Acquire::https::Timeout=30 \
      --no-install-recommends \
      "$@"; then
      return 0
    fi
    n=$((n + 1))
    echo "apt install failed (attempt ${n}/${max}); rotating mirror..." >&2
    sleep $((n * 2))
    debian_apt_setup || true
  done
  return 1
}

install_zig() {
  buildarch="${1:?build arch amd64|arm64}"
  zig_ver="${ZIG_VERSION:-0.13.0}"
  case "$buildarch" in
    amd64|x86_64) zig_arch=x86_64 ;;
    arm64|aarch64) zig_arch=aarch64 ;;
    *) echo "unsupported BUILDARCH for zig: ${buildarch}" >&2; return 1 ;;
  esac

  if command -v zig >/dev/null 2>&1; then
    echo "zig already installed: $(zig version)" >&2
    return 0
  fi

  url="https://ziglang.org/download/${zig_ver}/zig-linux-${zig_arch}-${zig_ver}.tar.xz"
  alt="https://github.com/ziglang/zig/releases/download/${zig_ver}/zig-linux-${zig_arch}-${zig_ver}.tar.xz"
  tmp="/tmp/zig-${zig_ver}.tar.xz"

  n=0
  while [ "$n" -lt 6 ]; do
    if curl -fsSL --retry 5 --retry-delay 2 -o "$tmp" "$url" \
      || curl -fsSL --retry 5 --retry-delay 2 -o "$tmp" "$alt"; then
      tar -xJf "$tmp" -C /opt
      ln -sf "/opt/zig-linux-${zig_arch}-${zig_ver}/zig" /usr/local/bin/zig
      zig version
      rm -f "$tmp"
      return 0
    fi
    n=$((n + 1))
    sleep $((n * 2))
  done
  echo "failed to download zig ${zig_ver}" >&2
  return 1
}

debian_apt_setup
