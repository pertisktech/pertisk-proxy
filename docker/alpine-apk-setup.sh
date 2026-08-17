#!/bin/sh
# Alpine apk mirror setup + retry helpers for flaky CDN/runners.
# Source from a Dockerfile RUN:  . /usr/local/sbin/alpine-apk-setup.sh
# After sourcing: repos are updated and apk_add_retry is available.
set -eu

ALPINE_VER="${ALPINE_VER:-v3.21}"

MIRRORS="
https://dl-cdn.alpinelinux.org/alpine
https://alpine.global.ssl.fastly.net/alpine
https://mirror.math.princeton.edu/pub/alpinelinux
https://mirrors.edge.kernel.org/alpine
https://mirror.csclub.uwaterloo.ca/alpine
"

apk_update_ok() {
  if apk update >/tmp/apk-update.log 2>&1; then
    return 0
  fi
  cat /tmp/apk-update.log >&2 || true
  return 1
}

try_mirror() {
  mirror="$1"
  printf '%s\n' \
    "${mirror}/${ALPINE_VER}/main" \
    "${mirror}/${ALPINE_VER}/community" > /etc/apk/repositories
  echo "apk: trying mirror ${mirror}" >&2
  apk_update_ok
}

alpine_apk_setup() {
  if [ -f /etc/apk/repositories ]; then
    sed -i -e 's|^[#[:space:]]*\(.*\/community\)|\1|' /etc/apk/repositories || true
  fi

  if apk_update_ok; then
    return 0
  fi

  for mirror in $MIRRORS; do
    if try_mirror "$mirror"; then
      return 0
    fi
    sleep 2
  done

  echo "apk: all mirrors failed for ${ALPINE_VER}" >&2
  return 1
}

# Retry apk add across transient CDN / index errors; rotates mirrors on repeated failure.
apk_add_retry() {
  n=0
  max=8
  while [ "$n" -lt "$max" ]; do
    if apk add --no-cache "$@"; then
      return 0
    fi
    n=$((n + 1))
    echo "apk add failed (attempt ${n}/${max}); refreshing index and retrying..." >&2
    sleep $((n * 2))
    if ! apk_update_ok; then
      alpine_apk_setup || true
    fi
  done
  return 1
}

alpine_apk_setup
