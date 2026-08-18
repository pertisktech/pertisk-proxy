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
           pertisk-tunnel-client (homelab / laptop)
                 │
                 ▼
           127.0.0.1:3000  or  https://10.1.1.195:8006 (Proxmox, etc.)
```

The tunnel is **raw TCP**. HTTPS backends (Proxmox) work when the Site upstream is also `https://127.0.0.1:<remote_port>` so TLS bytes pass through end-to-end.

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

## Local / LAN: tunnel client

```bash
cp tunnel/examples/client.toml ~/.config/pertisk-tunnel/client.toml
# set server = "your-vps.example.com:7000", same token, local/target = backend
pertisk-tunnel-client --config ~/.config/pertisk-tunnel/client.toml
```

`insecure_skip_verify = true` is required for the server’s ephemeral self-signed QUIC cert (token still required).

`local` (alias `target`) can be any dialable address on the client network:

| Backend | `local` / `target` |
|---------|-------------------|
| App on laptop | `127.0.0.1:3000` |
| Proxmox HTTPS | `https://10.1.1.195:8006` or `10.1.1.195:8006` |
| Other HTTP on LAN | `http://192.168.1.50:8080` |

## pertisk-proxy Site

In Admin → **Sites**, add:

| Field | HTTP app | HTTPS backend (Proxmox) |
|-------|----------|-------------------------|
| Domain | `dev.example.com` | `pve.example.com` |
| Upstream | `http://127.0.0.1:18080` | `https://127.0.0.1:18080` |
| SSL | Generate (HTTP-01 or DNS-01) | Generate |

Match `remote_port` in `server.toml` to the Site upstream port. For HTTPS backends, the Site **must** use `https://127.0.0.1:…` (pertisk-proxy skips upstream cert verify for self-signed Proxmox certs).

### Example: Proxmox

**VPS `server.toml`**

```toml
[[tunnels]]
name = "proxmox"
remote_port = 18006
```

**Client `client.toml`**

```toml
[[tunnels]]
name = "proxmox"
local = "https://10.1.1.195:8006"
```

**Site:** domain `pve.example.com`, upstream `https://127.0.0.1:18006`, enable SSL on the site.

## Status in Admin UI

Tunnel server serves `http://127.0.0.1:7700/status` by default.

pertisk-proxy can proxy that for the admin (authenticated):

```bash
# /etc/pertisk-proxy/pertisk-proxy.conf
PERTISK_TUNNEL_STATUS_URL=http://127.0.0.1:7700/status
```

Then open Admin → **Tunnels**.

## Bandwidth (VPS fair use)

Tunnel keepalives are tiny (a ping every 20s). They cannot explain ~1 TB/day.

**Tunneled site traffic is counted twice on the VPS:**

1. Internet → pertisk-proxy `:443` (WAN in)
2. tunnel QUIC UDP `:7000` → homelab (WAN out)

So ~500 GB through a public Site that uses a tunnel ≈ ~1 TB “total traffic” on many VPS plans.

Typical heavy sources: Proxmox UI (ISO upload, backup, noVNC/SPICE), large file Sites, bots hitting an exposed panel.

Check live counters (since `pertisk-tunnel-server` start):

```bash
curl -s http://127.0.0.1:7700/status | jq
# or Admin → Tunnels (Traffic column)
```

```bash
# UDP 7000 vs HTTPS right now
ss -uap | grep 7000 || true
iftop -nP -f 'port 7000 or port 443'
```


- Strong tunnel **token** (not admin password)
- Public service ports bind **loopback only** on the VPS
- Open firewall for UDP control port only
- Public TLS remains on pertisk-proxy; do not terminate internet TLS on the tunnel
