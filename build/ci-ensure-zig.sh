#!/usr/bin/env bash
# Ensure zig (+ optional cargo-zigbuild) is on PATH for host cross-compiles.
# Installs under $HOME/.local (no root) when missing.
set -euo pipefail

ZIG_VERSION="${ZIG_VERSION:-0.13.0}"

case "$(uname -m)" in
  x86_64|amd64) ZIG_ARCH=x86_64 ;;
  aarch64|arm64) ZIG_ARCH=aarch64 ;;
  *) echo "unsupported arch for zig: $(uname -m)" >&2; exit 1 ;;
esac

if ! command -v zig >/dev/null 2>&1; then
  PREFIX="${HOME}/.local/zig"
  DIR="${PREFIX}/zig-linux-${ZIG_ARCH}-${ZIG_VERSION}"
  mkdir -p "$PREFIX"
  if [ ! -x "${DIR}/zig" ]; then
    TMP="$(mktemp)"
    URL="https://ziglang.org/download/${ZIG_VERSION}/zig-linux-${ZIG_ARCH}-${ZIG_VERSION}.tar.xz"
    ALT="https://github.com/ziglang/zig/releases/download/${ZIG_VERSION}/zig-linux-${ZIG_ARCH}-${ZIG_VERSION}.tar.xz"
    echo "Downloading zig ${ZIG_VERSION} (${ZIG_ARCH})..."
    if ! curl -fsSL --retry 5 --retry-delay 2 -o "$TMP" "$URL"; then
      curl -fsSL --retry 5 --retry-delay 2 -o "$TMP" "$ALT"
    fi
    tar -xJf "$TMP" -C "$PREFIX"
    rm -f "$TMP"
  fi
  export PATH="${DIR}:${PATH}"
  # Persist for subsequent steps in the same job when sourced via GITHUB_PATH
  if [ -n "${GITHUB_PATH:-}" ]; then
    echo "${DIR}" >> "$GITHUB_PATH"
  fi
fi

command -v zig >/dev/null 2>&1 || {
  echo "zig not found after install" >&2
  exit 1
}
echo "zig $(zig version)"

if [ "${INSTALL_CARGO_ZIGBUILD:-1}" = "1" ]; then
  if ! command -v cargo-zigbuild >/dev/null 2>&1; then
    echo "Installing cargo-zigbuild..."
    cargo install cargo-zigbuild --locked
  fi
  cargo zigbuild --version 2>/dev/null || cargo zigbuild -V 2>/dev/null || true
fi
