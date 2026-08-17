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

## Build

From the repo root:

```bash
cargo build -p pertisk-tunnel-server -p pertisk-tunnel-client --release
# binaries: target/release/pertisk-tunnel-server
#           target/release/pertisk-tunnel-client
```

Or: `make tunnel`

## VPS: tunnel server

1. Copy example config and set a **strong random token**:

```bash
sudo mkdir -p /etc/pertisk-tunnel
sudo cp tunnel/examples/server.toml /etc/pertisk-tunnel/server.toml
# edit token + [[tunnels]] remote_port
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
