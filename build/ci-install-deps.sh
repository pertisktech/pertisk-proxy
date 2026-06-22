#!/usr/bin/env bash
# Install system packages required to compile pertisk-proxy on a bare Linux runner.
# Matches docker/Dockerfile.release (cmake, OpenSSL, clang, etc.).

set -euo pipefail

deps_ok() {
  command -v cmake >/dev/null 2>&1 \
    && command -v pkg-config >/dev/null 2>&1 \
    && pkg-config --exists openssl
}

if deps_ok; then
  echo "Native build dependencies already present"
  cmake --version | head -n1
  exit 0
fi

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif sudo -n true 2>/dev/null; then
  SUDO="sudo -n"
else
  echo "::error::Missing build deps (cmake/pkg-config/openssl) and runner has no passwordless sudo." >&2
  echo "Install on the runner, e.g.: apt-get install -y build-essential cmake pkg-config libssl-dev perl clang libclang-dev golang-go" >&2
  exit 1
fi

PKGS=(
  build-essential
  cmake
  pkg-config
  libssl-dev
  perl
  clang
  libclang-dev
  golang-go
)

if command -v apt-get >/dev/null 2>&1; then
  $SUDO apt-get update
  $SUDO apt-get install -y "${PKGS[@]}"
elif command -v dnf >/dev/null 2>&1; then
  $SUDO dnf install -y gcc gcc-c++ make cmake pkgconfig openssl-devel perl clang clang-devel golang
elif command -v yum >/dev/null 2>&1; then
  $SUDO yum install -y gcc gcc-c++ make cmake pkgconfig openssl-devel perl clang clang-devel golang
else
  echo "::error::No supported package manager (apt-get/dnf/yum)" >&2
  exit 1
fi

for cmd in cmake pkg-config cc; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "::error::Required tool still missing after install: $cmd" >&2
    exit 1
  fi
done

cmake --version | head -n1
