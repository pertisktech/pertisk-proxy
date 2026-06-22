#!/bin/sh
# Robust apt install for Debian/Ubuntu Docker builds on CI runners.
# Fixes intermittent "invalid signature" / "repository is not signed" when apt
# lists are served from a stale Docker layer cache or HTTP mirrors.
set -eu
export DEBIAN_FRONTEND=noninteractive

rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/partial/*

if [ -f /etc/apt/sources.list.d/debian.sources ]; then
  sed -i \
    -e 's|http://deb.debian.org|https://deb.debian.org|g' \
    -e 's|http://security.debian.org|https://security.debian.org|g' \
    /etc/apt/sources.list.d/debian.sources
elif [ -f /etc/apt/sources.list ]; then
  sed -i \
    -e 's|http://archive.ubuntu.com|https://archive.ubuntu.com|g' \
    -e 's|http://security.ubuntu.com|https://security.ubuntu.com|g' \
    -e 's|http://deb.debian.org|https://deb.debian.org|g' \
    /etc/apt/sources.list
fi

apt-get update \
  -o Acquire::AllowReleaseInfoChange=true \
  -o Acquire::Check-Valid-Until=false

apt-get install -y --no-install-recommends "$@"
rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/*
