# pertisk-tunnel

First-party reverse tunnel for [pertisk-proxy](../README.md).

| Binary | Role |
|--------|------|
| `pertisk-tunnel-server` | Runs on the VPS; QUIC UDP control; opens `127.0.0.1:<port>` |
| `pertisk-tunnel-client` | Runs on laptop/homelab; dials VPS; forwards to localhost **or LAN** (`https://10.x:8006` Proxmox, etc.) |

See [docs/tunnel.md](../docs/tunnel.md) for setup with Sites + ACME.

```bash
cargo build -p pertisk-tunnel-server -p pertisk-tunnel-client --release
# or: make tunnel
```

Examples: [examples/](examples/).
