#!/usr/bin/env bash
# Install system packages required to compile pertisk-proxy on a bare Linux runner.
# Supports Debian/Ubuntu (apt) and RHEL/AlmaLinux/Rocky (dnf/yum).
#
# Self-hosted runners without passwordless sudo: install once as root, e.g.
#   sudo dnf install -y gcc gcc-c++ make cmake pkgconf-pkg-config openssl-devel \
#     perl clang clang-devel golang
# Or set CI_SKIP_NATIVE_DEPS=1 and run tests via Docker (see test.yml).

set -euo pipefail

deps_ok() {
  command -v cmake >/dev/null 2>&1 \
    && command -v pkg-config >/dev/null 2>&1 \
    && (pkg-config --exists openssl || pkg-config --exists libssl)
}

almalinux_hint() {
  cat >&2 <<'EOF'
Install on the AlmaLinux / RHEL runner (as root), then re-run CI:

  dnf install -y gcc gcc-c++ make cmake pkgconf-pkg-config openssl-devel \
    perl clang clang-devel golang

Or grant the runner passwordless sudo for dnf, or set CI_SKIP_NATIVE_DEPS=1
and use the Docker-based test path.
EOF
}

debian_hint() {
  cat >&2 <<'EOF'
Install on the Debian / Ubuntu runner (as root), then re-run CI:

  apt-get update && apt-get install -y build-essential cmake pkg-config \
    libssl-dev perl clang libclang-dev golang-go
EOF
}

if [ "${CI_SKIP_NATIVE_DEPS:-0}" = "1" ]; then
  echo "CI_SKIP_NATIVE_DEPS=1 — skipping native dependency install"
  exit 0
fi

if deps_ok; then
  echo "Native build dependencies already present"
  cmake --version | head -n1
  pkg-config --modversion openssl 2>/dev/null || pkg-config --modversion libssl 2>/dev/null || true
  exit 0
fi

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif sudo -n true 2>/dev/null; then
  SUDO="sudo -n"
else
  echo "::error::Missing build deps (cmake/pkg-config/openssl) and runner has no passwordless sudo." >&2
  if command -v dnf >/dev/null 2>&1 || command -v yum >/dev/null 2>&1 \
    || [ -f /etc/almalinux-release ] || [ -f /etc/redhat-release ]; then
    almalinux_hint
  else
    debian_hint
  fi
  exit 1
fi

if command -v apt-get >/dev/null 2>&1; then
  $SUDO apt-get update
  $SUDO apt-get install -y \
    build-essential cmake pkg-config libssl-dev perl clang libclang-dev golang-go
elif command -v dnf >/dev/null 2>&1; then
  # AlmaLinux / RHEL / Rocky / Fedora
  $SUDO dnf install -y \
    gcc gcc-c++ make cmake pkgconf-pkg-config openssl-devel \
    perl clang clang-devel golang
elif command -v yum >/dev/null 2>&1; then
  $SUDO yum install -y \
    gcc gcc-c++ make cmake pkgconfig openssl-devel \
    perl clang clang-devel golang
else
  echo "::error::No supported package manager (apt-get/dnf/yum)" >&2
  exit 1
fi

if ! deps_ok; then
  echo "::error::Build deps still missing after install (cmake / pkg-config / openssl)." >&2
  exit 1
fi

for cmd in cmake pkg-config cc; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "::error::Required tool still missing after install: $cmd" >&2
    exit 1
  fi
done

cmake --version | head -n1
