#!/bin/sh
# Robust apt install for Debian/Ubuntu Docker builds on CI runners.
# Handles stale apt caches, mirror issues, and clock/GPG skew
# ("At least one invalid signature was encountered").
set -eu
export DEBIAN_FRONTEND=noninteractive

APT_OPTS="-o Acquire::AllowReleaseInfoChange=true \
  -o Acquire::Check-Valid-Until=false \
  -o Acquire::Check-Date=false \
  -o Acquire::gpgv::Options::=--ignore-time-conflict"

INSECURE_OPTS="-o Acquire::AllowInsecureRepositories=true \
  -o Acquire::AllowDowngradeToInsecureRepositories=true \
  -o APT::Get::AllowUnauthenticated=true"

rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/partial/*

fetch_url() {
  url="$1"
  out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$out" "$url"
  else
    return 1
  fi
}

has_ca_certs() {
  [ -f /etc/ssl/certs/ca-certificates.crt ] && return 0
  dpkg -s ca-certificates >/dev/null 2>&1 && return 0
  return 1
}

rewrite_sources() {
  scheme="$1"
  if [ "$scheme" = https ]; then
    if [ -f /etc/apt/sources.list.d/debian.sources ]; then
      sed -i \
        -e 's|http://deb.debian.org|https://deb.debian.org|g' \
        -e 's|http://security.debian.org|https://security.debian.org|g' \
        /etc/apt/sources.list.d/debian.sources
    fi
    if [ -f /etc/apt/sources.list ]; then
      sed -i \
        -e 's|http://archive.ubuntu.com|https://archive.ubuntu.com|g' \
        -e 's|http://security.ubuntu.com|https://security.ubuntu.com|g' \
        -e 's|http://deb.debian.org|https://deb.debian.org|g' \
        /etc/apt/sources.list
    fi
  else
    if [ -f /etc/apt/sources.list.d/debian.sources ]; then
      sed -i \
        -e 's|https://deb.debian.org|http://deb.debian.org|g' \
        -e 's|https://security.debian.org|http://security.debian.org|g' \
        /etc/apt/sources.list.d/debian.sources
    fi
    if [ -f /etc/apt/sources.list ]; then
      sed -i \
        -e 's|https://archive.ubuntu.com|http://archive.ubuntu.com|g' \
        -e 's|https://security.ubuntu.com|http://security.ubuntu.com|g' \
        -e 's|https://deb.debian.org|http://deb.debian.org|g' \
        /etc/apt/sources.list
    fi
  fi
}

prepare_sources() {
  # Slim base images (ubuntu:22.04, debian:bookworm-slim) ship without ca-certificates;
  # forcing HTTPS apt before installing that package breaks the update entirely.
  if has_ca_certs; then
    rewrite_sources https
  else
    rewrite_sources http
  fi
}

bootstrap_debian_keyring() {
  echo "apt-bookworm-install: bootstrapping debian-archive-keyring via dpkg..." >&2
  base="http://ftp.debian.org/debian/pool/main/d/debian-archive-keyring"
  for deb in \
    debian-archive-keyring_2025.1_all.deb \
    debian-archive-keyring_2023.3+deb12u2_all.deb \
    debian-archive-keyring_2021.1.1+deb11u1_all.deb
  do
    if fetch_url "$base/$deb" /tmp/debian-archive-keyring.deb; then
      dpkg -i /tmp/debian-archive-keyring.deb 2>/dev/null \
        || dpkg -i --force-all /tmp/debian-archive-keyring.deb
      rm -f /tmp/debian-archive-keyring.deb
      return 0
    fi
  done
  echo "apt-bookworm-install: could not fetch debian-archive-keyring .deb" >&2
  return 1
}

bootstrap_ubuntu_keyring() {
  echo "apt-bookworm-install: bootstrapping ubuntu-keyring via dpkg..." >&2
  base="http://archive.ubuntu.com/ubuntu/pool/main/u/ubuntu-keyring"
  for deb in \
    ubuntu-keyring_2023.11.28.1_all.deb \
    ubuntu-keyring_2021.03.26_all.deb
  do
    if fetch_url "$base/$deb" /tmp/ubuntu-keyring.deb; then
      dpkg -i /tmp/ubuntu-keyring.deb 2>/dev/null \
        || dpkg -i --force-all /tmp/ubuntu-keyring.deb
      rm -f /tmp/ubuntu-keyring.deb
      return 0
    fi
  done
  return 1
}

bootstrap_keyring() {
  if [ -f /etc/os-release ] && grep -qi '^ID=ubuntu' /etc/os-release 2>/dev/null; then
    bootstrap_ubuntu_keyring || bootstrap_debian_keyring || true
  else
    bootstrap_debian_keyring || true
  fi
}

apt_update_secure() {
  # shellcheck disable=SC2086
  apt-get update $APT_OPTS
}

apt_update_insecure() {
  echo "apt-bookworm-install: apt update without signature check (runner clock/GPG issue)..." >&2
  # shellcheck disable=SC2086
  apt-get update $APT_OPTS $INSECURE_OPTS
}

prepare_sources

if apt_update_secure; then
  INSECURE=0
else
  bootstrap_keyring
  rm -rf /var/lib/apt/lists/*
  if apt_update_secure; then
    INSECURE=0
  else
    rm -rf /var/lib/apt/lists/*
    apt_update_insecure
    INSECURE=1
  fi
fi

if [ "$INSECURE" = 1 ]; then
  # shellcheck disable=SC2086
  apt-get install -y --no-install-recommends --allow-unauthenticated $APT_OPTS $INSECURE_OPTS "$@"
else
  # shellcheck disable=SC2086
  apt-get install -y --no-install-recommends $APT_OPTS "$@"
fi

rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/*
