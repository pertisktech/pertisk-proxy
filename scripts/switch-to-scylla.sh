#!/usr/bin/env bash
# On the target host: migrate SQLite admin DB → Scylla and switch pertisk-proxy.
#
# Usage (as root on the proxy host):
#   ./scripts/switch-to-scylla.sh
#   SQLITE_PATH=/var/lib/pertisk-proxy/proxy.sqlite ./scripts/switch-to-scylla.sh
#
# If Scylla PasswordAuthenticator is enabled and credentials are unknown, this
# script briefly switches to AllowAllAuthenticator, resets the cassandra role
# password, creates a pertisk login role, migrates, then restores password auth.
set -euo pipefail

SQLITE_PATH="${SQLITE_PATH:-/var/lib/pertisk-proxy/proxy.sqlite}"
CONF="${CONF:-/etc/pertisk-proxy/pertisk-proxy.conf}"
UNIT_DROPIN_DIR="/etc/systemd/system/pertisk-proxy.service.d"
SCYLLA_YAML="${SCYLLA_YAML:-/etc/scylla/scylla.yaml}"
KEYSPACE="${PERTISK_SCYLLA_KEYSPACE:-pertisk_proxy}"
SCYLLA_USER="${PERTISK_SCYLLA_USER:-pertisk}"
# Prefer an explicit listen/broadcast address from scylla.yaml; fall back to hostname -I.
SCYLLA_HOST="${PERTISK_SCYLLA_HOSTS:-}"
if [ -z "$SCYLLA_HOST" ] && [ -f "$SCYLLA_YAML" ]; then
  _listen="$(awk -F': ' '/^listen_address:/{print $2; exit}' "$SCYLLA_YAML" | tr -d '[:space:]')"
  if [ -n "$_listen" ] && [ "$_listen" != "0.0.0.0" ] && [ "$_listen" != "localhost" ]; then
    SCYLLA_HOST="${_listen}:9042"
  fi
fi
SCYLLA_HOST="${SCYLLA_HOST:-127.0.0.1:9042}"
SCYLLA_CQL_HOST="${SCYLLA_HOST%%:*}"
SCYLLA_CQL_PORT="${SCYLLA_HOST##*:}"
[ "$SCYLLA_CQL_PORT" = "$SCYLLA_HOST" ] && SCYLLA_CQL_PORT=9042

need_cmd() { command -v "$1" >/dev/null 2>&1 || { echo "missing: $1" >&2; exit 1; }; }
need_cmd systemctl
need_cmd cqlsh
need_cmd migrate-sqlite-to-scylla
need_cmd openssl

[ -f "$SQLITE_PATH" ] || { echo "SQLite not found: $SQLITE_PATH" >&2; exit 1; }
[ -f "$SCYLLA_YAML" ] || { echo "Scylla config not found: $SCYLLA_YAML" >&2; exit 1; }

if [ "$(id -u)" -ne 0 ]; then
  echo "run as root" >&2
  exit 1
fi

PASS_FILE="/etc/pertisk-proxy/scylla-credentials"
# Prefer explicit env, then proxy credentials file, then sibling pertisk-logs env.
if [ -z "${PERTISK_SCYLLA_PASSWORD:-}" ] && [ -f "$PASS_FILE" ]; then
  # shellcheck disable=SC1090
  . "$PASS_FILE"
fi
if [ -z "${PERTISK_SCYLLA_PASSWORD:-}" ] && [ -f /etc/pertisk-logs/pertisk-logs.env ]; then
  # shellcheck disable=SC1091
  set -a
  # shellcheck disable=SC1091
  . /etc/pertisk-logs/pertisk-logs.env
  set +a
  SCYLLA_USER="${PERTISK_SCYLLA_USER:-cassandra}"
fi
SCYLLA_USER="${PERTISK_SCYLLA_USER:-$SCYLLA_USER}"
SCYLLA_PASS="${PERTISK_SCYLLA_PASSWORD:-}"
CASSANDRA_PASS="${CASSANDRA_SUPERUSER_PASSWORD:-}"
NEED_AUTH_RESET=0
if [ -z "$SCYLLA_PASS" ]; then
  NEED_AUTH_RESET=1
  SCYLLA_PASS="$(openssl rand -base64 24 | tr -d '=+/' | cut -c1-24)"
  CASSANDRA_PASS="${CASSANDRA_PASS:-$(openssl rand -base64 24 | tr -d '=+/' | cut -c1-24)}"
fi

BACKUP_DIR="/var/lib/pertisk-proxy/migrate-backup-$(date +%Y%m%d%H%M%S)"
mkdir -p "$BACKUP_DIR"
cp -a "$SQLITE_PATH" "$BACKUP_DIR/"
cp -a "$SCYLLA_YAML" "$BACKUP_DIR/scylla.yaml"
cp -a "$CONF" "$BACKUP_DIR/pertisk-proxy.conf" 2>/dev/null || true
echo "backed up to $BACKUP_DIR"

echo "==> stopping pertisk-proxy"
systemctl stop pertisk-proxy || true

AUTH_WAS_PASSWORD=0
AUTH_WAS_CASSANDRA_AUTHORIZER=0
if grep -qE '^[[:space:]]*authenticator:[[:space:]]*PasswordAuthenticator' "$SCYLLA_YAML"; then
  AUTH_WAS_PASSWORD=1
fi
if grep -qE '^[[:space:]]*authorizer:[[:space:]]*CassandraAuthorizer' "$SCYLLA_YAML"; then
  AUTH_WAS_CASSANDRA_AUTHORIZER=1
fi

wait_cql() {
  local user="${1:-}" pass="${2:-}"
  for _ in $(seq 1 90); do
    if [ -n "$user" ]; then
      if cqlsh "$SCYLLA_CQL_HOST" "$SCYLLA_CQL_PORT" -u "$user" -p "$pass" -e "DESCRIBE KEYSPACES" >/dev/null 2>&1; then
        return 0
      fi
    else
      if cqlsh "$SCYLLA_CQL_HOST" "$SCYLLA_CQL_PORT" -e "DESCRIBE KEYSPACES" >/dev/null 2>&1; then
        return 0
      fi
    fi
    sleep 2
  done
  return 1
}

restore_password_auth() {
  if [ "$AUTH_WAS_PASSWORD" -eq 1 ]; then
    sed -i 's/^[[:space:]]*authenticator:.*/authenticator: PasswordAuthenticator/' "$SCYLLA_YAML"
  fi
  if [ "$AUTH_WAS_CASSANDRA_AUTHORIZER" -eq 1 ]; then
    sed -i 's/^[[:space:]]*authorizer:.*/authorizer: CassandraAuthorizer/' "$SCYLLA_YAML"
  fi
  if [ "$AUTH_WAS_PASSWORD" -eq 1 ] || [ "$AUTH_WAS_CASSANDRA_AUTHORIZER" -eq 1 ]; then
    systemctl restart scylla-server
    wait_cql cassandra "$CASSANDRA_PASS" || wait_cql "$SCYLLA_USER" "$SCYLLA_PASS" || true
  fi
}

# Probe existing credentials before any AllowAll reset.
if [ -n "$SCYLLA_PASS" ] && wait_cql "$SCYLLA_USER" "$SCYLLA_PASS"; then
  echo "==> Scylla login ok as ${SCYLLA_USER}@${SCYLLA_HOST}"
  NEED_AUTH_RESET=0
elif [ "$AUTH_WAS_PASSWORD" -eq 1 ] || [ "$AUTH_WAS_CASSANDRA_AUTHORIZER" -eq 1 ]; then
  NEED_AUTH_RESET=1
fi

if [ "$NEED_AUTH_RESET" -eq 1 ]; then
  echo "==> temporarily enabling AllowAll authz (needed to reset roles)"
  CASSANDRA_PASS="${CASSANDRA_PASS:-$(openssl rand -base64 24 | tr -d '=+/' | cut -c1-24)}"
  sed -i 's/^[[:space:]]*authenticator:.*/authenticator: AllowAllAuthenticator/' "$SCYLLA_YAML"
  sed -i 's/^[[:space:]]*authorizer:.*/authorizer: AllowAllAuthorizer/' "$SCYLLA_YAML"
  systemctl restart scylla-server
  if ! wait_cql; then
    echo "Scylla did not become ready after AllowAll switch" >&2
    exit 1
  fi

  echo "==> resetting cassandra + creating ${SCYLLA_USER} role"
  cqlsh "$SCYLLA_CQL_HOST" "$SCYLLA_CQL_PORT" <<CQL
ALTER ROLE cassandra WITH PASSWORD = '${CASSANDRA_PASS}' AND LOGIN = true;
CREATE ROLE IF NOT EXISTS ${SCYLLA_USER} WITH PASSWORD = '${SCYLLA_PASS}' AND LOGIN = true;
ALTER ROLE ${SCYLLA_USER} WITH PASSWORD = '${SCYLLA_PASS}' AND LOGIN = true;
GRANT CREATE ON ALL KEYSPACES TO ${SCYLLA_USER};
CQL
fi

echo "==> migrating ${SQLITE_PATH} → ${SCYLLA_HOST} keyspace ${KEYSPACE}"
export PERTISK_SCYLLA_HOSTS="$SCYLLA_HOST"
export PERTISK_SCYLLA_KEYSPACE="$KEYSPACE"
if [ "$NEED_AUTH_RESET" -eq 1 ]; then
  # Still AllowAll for the migrate binary connection.
  unset PERTISK_SCYLLA_USER PERTISK_SCYLLA_PASSWORD || true
else
  export PERTISK_SCYLLA_USER="$SCYLLA_USER"
  export PERTISK_SCYLLA_PASSWORD="$SCYLLA_PASS"
fi
migrate-sqlite-to-scylla "$SQLITE_PATH"

if [ "$NEED_AUTH_RESET" -eq 1 ]; then
  echo "==> granting ${SCYLLA_USER} on ${KEYSPACE}"
  cqlsh "$SCYLLA_CQL_HOST" "$SCYLLA_CQL_PORT" <<CQL
GRANT ALL PERMISSIONS ON KEYSPACE ${KEYSPACE} TO ${SCYLLA_USER};
CQL
  echo "==> restoring PasswordAuthenticator"
  restore_password_auth
fi

umask 077
cat > "$PASS_FILE" <<EOF
PERTISK_SCYLLA_USER=${SCYLLA_USER}
PERTISK_SCYLLA_PASSWORD=${SCYLLA_PASS}
EOF
chown root:pertisk-proxy "$PASS_FILE" 2>/dev/null || chown root:root "$PASS_FILE"
chmod 640 "$PASS_FILE"

# Persist cassandra superuser reset password for ops (root-only).
umask 077
cat > /root/scylla-cassandra-superuser <<EOF
# Written by switch-to-scylla.sh — keep secret
CASSANDRA_USER=cassandra
CASSANDRA_PASSWORD=${CASSANDRA_PASS}
EOF
chmod 600 /root/scylla-cassandra-superuser

echo "==> updating ${CONF}"
# Remove prior backend lines then append Scylla block.
tmp="$(mktemp)"
grep -vE '^[[:space:]]*(PERTISK_DB_BACKEND|PERTISK_SCYLLA_|PERTISK_DB_PATH)=' "$CONF" > "$tmp" || true
cat >> "$tmp" <<EOF

# Switched by switch-to-scylla.sh ($(date -u +%Y-%m-%dT%H:%M:%SZ))
PERTISK_DB_BACKEND=scylla
PERTISK_SCYLLA_HOSTS=${SCYLLA_HOST}
PERTISK_SCYLLA_KEYSPACE=${KEYSPACE}
PERTISK_SCYLLA_DC=datacenter1
PERTISK_SCYLLA_USER=${SCYLLA_USER}
PERTISK_SCYLLA_PASSWORD=${SCYLLA_PASS}
# Original SQLite kept at ${BACKUP_DIR}/
# PERTISK_DB_PATH=${SQLITE_PATH}
EOF
cp "$tmp" "$CONF"
rm -f "$tmp"
chown root:pertisk-proxy "$CONF" 2>/dev/null || true
chmod 640 "$CONF"

mkdir -p "$UNIT_DROPIN_DIR"
cat > "${UNIT_DROPIN_DIR}/20-scylla.conf" <<'DROPIN'
[Unit]
After=scylla-server.service
Wants=scylla-server.service
DROPIN

systemctl daemon-reload
echo "==> starting pertisk-proxy"
systemctl enable --now pertisk-proxy
systemctl restart pertisk-proxy
sleep 2
systemctl is-active --quiet pertisk-proxy
systemctl status pertisk-proxy --no-pager | head -20

echo
echo "Done. Admin DB is Scylla (${SCYLLA_HOST} / ${KEYSPACE})."
echo "Credentials: ${PASS_FILE} (and cassandra superuser in /root/scylla-cassandra-superuser)"
echo "SQLite backup: ${BACKUP_DIR}"
