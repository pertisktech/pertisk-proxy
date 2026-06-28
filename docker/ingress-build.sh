#!/bin/sh
# Build pertisk-proxy-ingress inside docker/Dockerfile.ingress (BUILDPLATFORM only).
# Cross-compilation uses cargo-zigbuild (any host arch). musl.cc -cross toolchains are
# x86_64-hosted and cannot run on arm64 build hosts.
set -eu

# TARGETARCH is passed via --build-arg; derive from TARGETPLATFORM if missing.
if [ -z "${TARGETARCH:-}" ]; then
  case "${TARGETPLATFORM:-}" in
    linux/arm64|linux/arm64/*) TARGETARCH=arm64 ;;
    linux/amd64|linux/amd64/*) TARGETARCH=amd64 ;;
  esac
fi

if [ -z "${TARGETARCH:-}" ]; then
  echo "ingress-build: TARGETARCH is required (use buildx --platform or --build-arg TARGETARCH=...)" >&2
  exit 1
fi

echo "ingress-build: BUILDARCH=${BUILDARCH:-?} TARGETARCH=${TARGETARCH} TARGETPLATFORM=${TARGETPLATFORM:-?}"

JOBS="${CARGO_BUILD_JOBS:-$(nproc)}"
export CARGO_BUILD_JOBS="${JOBS}"

case "${TARGETARCH}" in
  amd64) RUST_TARGET=x86_64-unknown-linux-musl ;;
  arm64) RUST_TARGET=aarch64-unknown-linux-musl ;;
  *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;;
esac

rustup target add "${RUST_TARGET}"

if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then
  cargo zigbuild --release --locked --target "${RUST_TARGET}" \
    --bin pertisk-proxy-ingress --features ingress
else
  cargo build --release --locked --target "${RUST_TARGET}" \
    --bin pertisk-proxy-ingress --features ingress
fi
TARGET_DIR="/app/target/${RUST_TARGET}/release"

cp "${TARGET_DIR}/pertisk-proxy-ingress" /app/pertisk-proxy-ingress

file_out="$(file /app/pertisk-proxy-ingress)"
echo "ingress-build: binary=${file_out}"
case "${TARGETARCH}" in
  amd64) echo "$file_out" | grep -Eq 'x86-64|Intel 80386' || { echo "wrong arch for amd64: $file_out" >&2; exit 1; } ;;
  arm64) echo "$file_out" | grep -Eq 'aarch64|ARM' || { echo "wrong arch for arm64: $file_out" >&2; exit 1; } ;;
esac

mkdir -p /runtime/etc/ssl/certs /runtime/lib /runtime/usr/local/bin /runtime/usr/share/pertisk-proxy/admin/dist
apk add --no-cache ca-certificates
cp /etc/ssl/certs/ca-certificates.crt /runtime/etc/ssl/certs/

if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then
  case "${TARGETARCH}" in
    amd64) native_pkg=x86_64-linux-musl-native ;;
    arm64) native_pkg=aarch64-linux-musl-native ;;
  esac
  if [ ! -d "/opt/${native_pkg}" ]; then
    curl -fsSL "https://musl.cc/${native_pkg}.tgz" | tar xz -C /opt
  fi
  sysroot="/opt/${native_pkg}/${native_pkg%-native}/lib"
  cp -a "${sysroot}/." /runtime/lib/
else
  apk add --no-cache openssl
  for d in /lib /usr/lib; do
    for pat in libssl.so* libcrypto.so* ld-musl-*.so*; do
      cp -a ${d}/${pat} /runtime/lib/ 2>/dev/null || true
    done
  done
fi

cp /app/pertisk-proxy-ingress /runtime/usr/local/bin/pertisk-proxy-ingress
cp -a /app/admin/dist/. /runtime/usr/share/pertisk-proxy/admin/dist/
