# Shared deploy host parsing for build/deploy-*.sh
# DEPLOY_HOST=user@host overrides REMOTE_USER + REMOTE_HOST when set.

resolve_deploy_host() {
  if [ -n "${DEPLOY_HOST:-}" ]; then
    if [[ "$DEPLOY_HOST" != *@* ]]; then
      echo "DEPLOY_HOST must be user@host (got: $DEPLOY_HOST)" >&2
      return 1
    fi
    REMOTE_USER="${DEPLOY_HOST%%@*}"
    REMOTE_HOST="${DEPLOY_HOST#*@}"
    export REMOTE_USER REMOTE_HOST
  fi
}

deploy_ssh_target() {
  if [ -n "${DEPLOY_HOST:-}" ]; then
    echo "$DEPLOY_HOST"
  elif [ -n "${REMOTE_HOST:-}" ]; then
    echo "${REMOTE_USER:-root}@${REMOTE_HOST}"
  else
    return 1
  fi
}

detect_remote_machine_arch() {
  local target="${1:?ssh target required}"
  local ssh_opts="${2:-}"
  local uname_m
  # shellcheck disable=SC2086
  uname_m=$(ssh $ssh_opts "$target" 'uname -m' 2>/dev/null) || {
    echo "Failed to detect remote architecture via SSH ($target)" >&2
    return 1
  }
  case "$uname_m" in
    x86_64|amd64) echo amd64 ;;
    aarch64|arm64) echo arm64 ;;
    *)
      echo "Unsupported remote architecture: $uname_m" >&2
      return 1
      ;;
  esac
}

# When DEPLOY_ARCH=auto, SSH to the deploy host and set DEPLOY_ARCH to amd64 or arm64.
resolve_deploy_arch() {
  if [ "${DEPLOY_ARCH:-amd64}" != "auto" ]; then
    return 0
  fi
  local target
  target="$(deploy_ssh_target)" || {
    echo "DEPLOY_ARCH=auto requires DEPLOY_HOST or REMOTE_HOST" >&2
    return 1
  }
  DEPLOY_ARCH="$(detect_remote_machine_arch "$target" "${DEPLOY_SSH_OPTS:-}")" || return 1
  export DEPLOY_ARCH
  echo "Detected remote architecture: $DEPLOY_ARCH ($target)"
}
