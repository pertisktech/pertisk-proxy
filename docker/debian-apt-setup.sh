#!/bin/sh
# Debian apt mirror setup + retry helpers for flaky CDN/runners.
# Source from a Dockerfile RUN:  . /usr/local/sbin/debian-apt-setup.sh
set -eu

# bookworm
DEBIAN_SUITE="${DEBIAN_SUITE:-bookworm}"

MIRRORS="
http://deb.debian.org/debian
https://deb.debian.org/debian
http://mirrors.aliyun.com/debian
https://mirrors.aliyun.com/debian
http://mirror.csclub.uwaterloo.ca/debian
https://mirrors.kernel.org/debian
"

apt_update_ok() {
  if apt-get update -o Acquire::Retries=5 >/tmp/apt-update.log 2>&1; then
    return 0
  fi
  cat /tmp/apt-update.log >&2 || true
  return 1
}

try_mirror() {
  mirror="$1"
  # Keep security on Debian's official host; suite mirrors vary by provider.
  printf '%s\n' \
    "deb ${mirror} ${DEBIAN_SUITE} main" \
    "deb ${mirror} ${DEBIAN_SUITE}-updates main" \
    "deb http://security.debian.org/debian-security ${DEBIAN_SUITE}-security main" \
    > /etc/apt/sources.list
  # Clear deb822 sources that may still point at unreachable hosts.
  rm -f /etc/apt/sources.list.d/debian.sources /etc/apt/sources.list.d/*.sources 2>/dev/null || true
  echo "apt: trying mirror ${mirror}" >&2
  apt_update_ok
}

debian_apt_setup() {
  if apt_update_ok; then
    return 0
  fi

  for mirror in $MIRRORS; do
    if try_mirror "$mirror"; then
      return 0
    fi
    sleep 2
  done

  echo "apt: all mirrors failed for ${DEBIAN_SUITE}" >&2
  return 1
}

apt_install_retry() {
  n=0
  max=8
  while [ "$n" -lt "$max" ]; do
    if DEBIAN_FRONTEND=noninteractive apt-get install -y \
      -o Acquire::Retries=5 \
      --no-install-recommends \
      "$@"; then
      return 0
    fi
    n=$((n + 1))
    echo "apt install failed (attempt ${n}/${max}); refreshing and retrying..." >&2
    sleep $((n * 2))
    if ! apt_update_ok; then
      debian_apt_setup || true
    fi
  done
  return 1
}

# Install zig from upstream tarball (not distro packages).
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
