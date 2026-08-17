# Reverse tunnel (local → VPS → pertisk-proxy)

Expose a service on your laptop (or home LAN) through a public VPS that already runs **pertisk-proxy**. The local client dials **out** to the VPS (no inbound ports on your home network). Public HTTPS stays on pertisk-proxy (Sites + ACME).

```text
Internet ──▶ pertisk-proxy :80/:443
                 │
                 ▼
           127.0.0.1:18080   ◀── pertisk-tunnel-server (loopback only)
                 ▲
                 │ QUIC UDP :7000 (token auth)
                 │
           pertisk-tunnel-client (your laptop)
                 │
                 ▼
           127.0.0.1:3000 (your app)
```

## Build / packages

```bash
make tunnel                    # release binaries
make package-tunnel VERSION=0.1.80   # RPM+DEB for server and client (amd64)
make package-tunnel-arm64 VERSION=0.1.80

# Deploy to VPS / laptop
make deploy-rpm-tunnel DEPLOY_HOST=user@vps VERSION=0.1.80
make deploy-rpm-tunnel-server DEPLOY_HOST=user@vps
make deploy-rpm-tunnel-client DEPLOY_HOST=user@laptop
```

Packages install:

| Package | Binary | Config | Unit |
|---------|--------|--------|------|
| `pertisk-tunnel-server` | `/usr/bin/pertisk-tunnel-server` | `/etc/pertisk-tunnel/server.toml` | `pertisk-tunnel-server.service` |
| `pertisk-tunnel-client` | `/usr/bin/pertisk-tunnel-client` | `/etc/pertisk-tunnel/client.toml` | `pertisk-tunnel-client.service` |

After install: edit the token in the toml, then `sudo systemctl enable --now pertisk-tunnel-server` (VPS) or `…-client` (laptop).

## VPS: tunnel server

1. Copy example config and set a **strong random token**:

```bash
sudo mkdir -p /etc/pertisk-tunnel
sudo cp tunnel/examples/server.toml /etc/pertisk-tunnel/server.toml
# edit token + [[tunnels]] remote_port
```

**Tunnel names must match on both sides.** If the client has `admin-15` and `pertisk-15`, the VPS `server.toml` needs the same names:

```toml
token = "same-secret-as-client"

[[tunnels]]
name = "admin-15"
remote_port = 19080

[[tunnels]]
name = "pertisk-15"
remote_port = 18801
```

Then restart: `sudo systemctl restart pertisk-tunnel-server`

If the client logs `closed by peer` during hello, check server logs:

```bash
journalctl -u pertisk-tunnel-server -n 50 --no-pager
```

Typical causes: **token mismatch**, or **unknown tunnel name** (not listed on the server).

Loopback ports are bound **once at server start**. If you see `Address already in use` on `127.0.0.1:<port>`, something else owns that port:

```bash
ss -lntp | grep 18080
# change remote_port in server.toml, or stop the other process
sudo systemctl restart pertisk-tunnel-server
```

2. Firewall: allow **UDP 7000** (control). Do **not** expose `status_bind` (default `127.0.0.1:7700`). Keep management API on localhost if possible.

3. Run:

```bash
pertisk-tunnel-server --config /etc/pertisk-tunnel/server.toml
# or install tunnel/examples/pertisk-tunnel-server.service
```

4. When a client connects, the server listens on **`127.0.0.1:<remote_port>`** only.

## Local: tunnel client

```bash
cp tunnel/examples/client.toml ~/.config/pertisk-tunnel/client.toml
# set server = "your-vps.example.com:7000", same token, local = your app
pertisk-tunnel-client --config ~/.config/pertisk-tunnel/client.toml
```

`insecure_skip_verify = true` is required for the server’s ephemeral self-signed QUIC cert (token still required).

## pertisk-proxy Site

In Admin → **Sites**, add:

| Field | Example |
|-------|---------|
| Domain | `dev.example.com` |
| Upstream | `http://127.0.0.1:18080` |
| SSL | Generate (HTTP-01 or DNS-01) |

Match `remote_port` in `server.toml` to the Site upstream port.

## Status in Admin UI

Tunnel server serves `http://127.0.0.1:7700/status` by default.

pertisk-proxy can proxy that for the admin (authenticated):

```bash
# /etc/pertisk-proxy/pertisk-proxy.conf
PERTISK_TUNNEL_STATUS_URL=http://127.0.0.1:7700/status
```

Then open Admin → **Tunnels**.

## Security

- Strong tunnel **token** (not admin password)
- Public service ports bind **loopback only** on the VPS
- Open firewall for UDP control port only
- Public TLS remains on pertisk-proxy; do not terminate internet TLS on the tunnel
