#!/bin/sh
# Build pertisk-proxy-ingress inside docker/Dockerfile.ingress (BUILDPLATFORM only).
# Cross-compilation uses cargo-zigbuild (any host arch). musl.cc -cross toolchains are
# x86_64-hosted and cannot run on arm64 build hosts.
set -eu

case "${TARGETPLATFORM:-}" in
  linux/arm64|linux/arm64/*) TARGETARCH=arm64 ;;
  linux/amd64|linux/amd64/*) TARGETARCH=amd64 ;;
esac

JOBS="${CARGO_BUILD_JOBS:-$(nproc)}"
export CARGO_BUILD_JOBS="${JOBS}"

case "${TARGETARCH}" in
  amd64) RUST_TARGET=x86_64-unknown-linux-musl ;;
  arm64) RUST_TARGET=aarch64-unknown-linux-musl ;;
  *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;;
esac

if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then
  rustup target add "${RUST_TARGET}"
  cargo zigbuild --release --locked --target "${RUST_TARGET}" \
    --bin pertisk-proxy-ingress --features ingress
  TARGET_DIR="/app/target/${RUST_TARGET}/release"
else
  cargo build --release --locked --bin pertisk-proxy-ingress --features ingress
  TARGET_DIR="/app/target/release"
fi

cp "${TARGET_DIR}/pertisk-proxy-ingress" /app/pertisk-proxy-ingress

case "${TARGETARCH}" in
  amd64) file /app/pertisk-proxy-ingress | grep -Eq 'x86-64|Intel 80386' ;;
  arm64) file /app/pertisk-proxy-ingress | grep -Eq 'aarch64|ARM' ;;
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
