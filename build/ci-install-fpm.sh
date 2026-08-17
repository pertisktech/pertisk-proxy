#!/usr/bin/env bash
# Optionally install host fpm + rpmbuild on Alma/RHEL for faster packaging (no Docker).
# Safe to run repeatedly. Requires root or passwordless sudo.
set -euo pipefail

if command -v fpm >/dev/null 2>&1 && command -v rpmbuild >/dev/null 2>&1; then
  echo "Host fpm + rpmbuild already available"
  fpm --version | head -n1
  rpmbuild --version | head -n1
  exit 0
fi

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif sudo -n true 2>/dev/null; then
  SUDO="sudo -n"
else
  echo "No passwordless sudo — skip host fpm install (Docker packaging will be used)"
  exit 0
fi

if command -v dnf >/dev/null 2>&1; then
  $SUDO dnf -y install epel-release || true
  $SUDO dnf -y install ruby ruby-devel gcc make rpm-build cpio tar gzip which binutils xz
  if ! command -v fpm >/dev/null 2>&1; then
    $SUDO gem install --no-document fpm
  fi
elif command -v apt-get >/dev/null 2>&1; then
  $SUDO apt-get update
  $SUDO apt-get install -y ruby ruby-dev build-essential rpm cpio
  if ! command -v fpm >/dev/null 2>&1; then
    $SUDO gem install --no-document fpm
  fi
else
  echo "No dnf/apt — skip host fpm install"
  exit 0
fi

if command -v fpm >/dev/null 2>&1 && command -v rpmbuild >/dev/null 2>&1; then
  echo "Host fpm ready"
  fpm --version | head -n1
else
  echo "Host fpm still incomplete; Docker packaging remains available"
fi
